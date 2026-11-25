//! 候选词引擎(移植 engine.py):切分 → 单字+词组+混拼合并排序 → DP 整句智能首选。
//! 逻辑同源 Python,词库经 db.rs 读 ciku.db,前缀查询走 trie.rs 内存索引。

use crate::db::CikuDb;
use crate::syllable::Syllables;
use crate::trie::MemoryIndex;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// 索引缓存格式版本:序列化结构变更时 +1,旧缓存自动失效重建。
/// v2: trie 节点增加 top1(单字)通道,旧 v1 缓存无此字段,必须重建。
/// v3: MemoryIndex 增加混拼索引 jp_map/first_jp_map,必须重建。
const INDEX_CACHE_VERSION: u32 = 3;

/// 模糊音扩展字的权重降级量(2026-09-02):把 zhi 等模糊字压到原音字权重之下,
/// 保证打 zi 时原音字(自/字/…/耔/缁)排在模糊 zhi 字(只/指/值)之前。
/// 取值大于词库权重动态范围(实测 max≈26000),降级后模糊字恒为负,原音字(含 weight=0)恒在前。
const FUZZY_DEMOTE: i64 = 1_000_000;

/// 白名单加权量:白名单词在同等权重下排更前(对齐 Python professional_corrector 逻辑)。
const WHITELIST_BONUS: i64 = 10_000_000;

/// 索引缓存头部(自描述,校验用)。后接 bincode 序列化的 MemoryIndex。
#[derive(serde::Serialize, serde::Deserialize)]
struct IndexHeader {
    magic: [u8; 4], // b"PIXC"
    version: u32,
    db_len: u64,
    db_mtime_secs: u64,
}

/// 词库指纹:文件大小 + mtime。变了即认为词库更新,缓存失效重建。
fn db_fingerprint(db_path: &str) -> Option<(u64, u64)> {
    let md = std::fs::metadata(db_path).ok()?;
    let mtime = md
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some((md.len(), mtime))
}

/// 缓存文件路径:与词库同目录,`<name>.idx`。
fn cache_path_for(db_path: &str) -> PathBuf {
    let p = Path::new(db_path);
    p.with_extension("idx")
}

/// mmap 缓存路径:与词库同目录,`<name>.midx`(2026-09-02)。
fn mmap_path_for(db_path: &str) -> PathBuf {
    let p = Path::new(db_path);
    p.with_extension("midx")
}

/// (候选词, 权重)
pub type Candidate = (String, i64);

/// 九宫格查询结果：可能的读音 + 这些读音下的候选词。
#[derive(Default)]
pub struct T9Result {
    /// 数字串能拼出的读音（音节间空格分隔，如 "ni hao"）；拼不出时为空。
    pub pinyins: Vec<String>,
    /// 候选词（已按权重降序）。
    pub words: Vec<Candidate>,
}

pub struct ImeEngine {
    pub db: CikuDb,
    pub syll: Syllables,
    pub mem: Option<MemoryIndex>,
    /// 模糊音规则集(对齐 ime_config.FUZZY_RULES)。String 持有,支持从配置文件
    /// 动态读入(2026-09-04 模糊音设置功能,见 ffi.rs load_fuzzy_rules)。
    pub fuzzy_rules: Vec<String>,
    /// P0 智能标点状态(引号配对)。进程级:引擎句柄单例(FIXME 将来有 IC 概念时下沉)。
    /// 用 Mutex 是因为 FFI 只拿得到 &ImeEngine,无法 &mut。
    pub punct_state: std::sync::Mutex<crate::punct::PunctState>,
    /// P2 简繁输出模式：0=原样 1=简→繁 2=繁→简。
    /// 默认 0,可由 `<词库目录>/traditional.txt` 或 FFI 设置。
    pub script_mode: std::sync::atomic::AtomicU8,
}

/// P2 简繁输出模式。数值与 C ABI `prisir_ime_script` 一致。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScriptMode {
    /// 原样输出（默认）
    Off = 0,
    /// 简体 → 繁体
    S2T = 1,
    /// 繁体 → 简体
    T2S = 2,
}

impl ScriptMode {
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => ScriptMode::S2T,
            2 => ScriptMode::T2S,
            _ => ScriptMode::Off,
        }
    }

    pub fn as_i32(self) -> i32 {
        self as i32
    }

    /// 循环下一种模式：原样 → 简→繁 → 繁→简 → 原样（供 UI 按钮循环切换）。
    pub fn next(self) -> Self {
        match self {
            ScriptMode::Off => ScriptMode::S2T,
            ScriptMode::S2T => ScriptMode::T2S,
            ScriptMode::T2S => ScriptMode::Off,
        }
    }
}

/// 模糊音映射:规则名 -> [(源,目标)](对齐 Python _FUZZY_MAP)
fn fuzzy_map(rule: &str) -> &'static [(&'static str, &'static str)] {
    match rule {
        "z_zh" => &[("zh", "z"), ("z", "zh")],
        "c_ch" => &[("ch", "c"), ("c", "ch")],
        "s_sh" => &[("sh", "s"), ("s", "sh")],
        "an_ang" => &[("an", "ang"), ("ang", "an")],
        "en_eng" => &[("en", "eng"), ("eng", "en")],
        "in_ing" => &[("in", "ing"), ("ing", "in")],
        "ian_iang" => &[("ian", "iang"), ("iang", "ian")],
        "uan_uang" => &[("uan", "uang"), ("uang", "uan")],
        "ai_an" => &[("ai", "an"), ("an", "ai")],
        "un_ong" => &[("un", "ong"), ("ong", "un")],
        _ => &[],
    }
}
/// 声母规则(起始替换);其余按结尾替换
fn fuzzy_is_initial(rule: &str) -> bool {
    matches!(rule, "z_zh" | "c_ch" | "s_sh")
}

/// 简拼 = 每个音节首字母拼接(供 db.add_user_word 用)
pub fn to_jianpin(pinyin: &str) -> String {
    // pinyin 形如 "nihao" / "zhongguo";按音节切分取首字母
    let syll = Syllables::new();
    let segs = syll.segment(pinyin);
    if segs.is_empty() {
        // 兜底:空格/逐字符
        return pinyin.chars().filter(|c| c.is_ascii_alphabetic()).take(1).collect();
    }
    segs.iter().filter_map(|s| s.chars().next()).collect()
}

impl ImeEngine {
    pub fn new(db_path: &str) -> Result<Self, String> {
        let db = CikuDb::open(db_path).map_err(|e| format!("open db: {e}"))?;
        Ok(ImeEngine {
            db,
            syll: Syllables::new(),
            mem: None,
            fuzzy_rules: Vec::new(),
            punct_state: std::sync::Mutex::new(crate::punct::PunctState::default()),
            script_mode: std::sync::atomic::AtomicU8::new(0),
        })
    }

    /// 构建内存索引(一次性灌 phrase+pinyin 全量)。失败保留 None 走 SQLite。
    pub fn build_memory_index(&mut self) -> Result<(), String> {
        let mem = Self::rebuild_index(&self.db)?;
        self.mem = Some(mem);
        Ok(())
    }

    /// 从 DB 全量重建内存索引(不落盘)。
    fn rebuild_index(db: &CikuDb) -> Result<MemoryIndex, String> {
        let phrase = db.all_phrase_rows().map_err(|e| format!("phrase: {e}"))?;
        let phrase_jp = db.all_phrase_rows_with_jp().map_err(|e| format!("phrase_jp: {e}"))?;
        let pinyin = db.all_single_char_rows().map_err(|e| format!("pinyin: {e}"))?;
        let mut mem = MemoryIndex::new();
        for (key, value, weight) in &phrase {
            mem.insert(key, value, *weight);
        }
        // 混拼索引(jp 精确 + 反混),对齐原 SQLite query_phrase_mixed 三条路径。
        for (jp, key, value, weight) in &phrase_jp {
            mem.insert_mixed(jp, key, value, *weight);
        }
        for (key, value, weight) in &pinyin {
            mem.insert(key, value, *weight);
        }
        mem.finalize(); // 反混桶预排序(权重降序),查询 early-exit
        Ok(mem)
    }

    /// 加载或构建内存索引(索引持久化:第一次建好后存盘,之后启动直接反序列化)。
    ///
    /// 流程(2026-09-02 加 mmap 优先):
    ///   1. 先试 `<db>.midx`(mmap 紧凑格式):mmap 映射微秒级、零解析、多进程共享物理页,
    ///      校验 magic/version/db 指纹通过即用之(来源 "mmap")。
    ///   2. 否则读 `<db>.idx`(bincode):校验 指纹+格式版本,对上则用之(来源 "cache")。
    ///   3. 否则从 DB 重建,并(原子写)同时落盘 .idx 与 .midx(双写灰度,来源 "rebuilt")。
    /// 任何一步失败都回退到纯 SQLite(mem=None),不影响功能。
    /// 返回 (是否走内存索引, 来源描述) 供日志/诊断。
    pub fn load_or_build_index(&mut self, db_path: &str) -> (bool, &'static str) {
        // 2026-09-02 残页/跳页/回删卡顿 根因定位结论:mmap 内存索引层在某些时刻返回
        // 与 midx 实际内容不一致的数据,导致候选窗渲染出残页(位置4空白)、自动跳最后一页、
        // 回删卡顿。二分已证:强制走 SQLite 时这些现象全部消失。故彻底旁路内存索引,
        // 固定走 SQLite(唯一已验证干净的路径)。后续只剩「扩充词库」让 SQLite 查询够用。
        // mmap/.idx/trie 三层全部停用;mmap_index 模块保留但不再进入加载链。
        let _ = db_path;
        self.mem = None;
        (false, "sqlite")
    }

    /// **PC 侧工具用**：从 DB 全量重建内存索引并落盘成 `<db>.midx`。
    ///
    /// 设备上**不要**调用：全量重建要几十秒 + 上百 MB 内存，手机扛不住；
    /// 手机只负责映射已有的 `.midx`（见 `load_mmap_index`）。
    pub fn build_and_save_mmap_index(&mut self, db_path: &str) -> Result<usize, String> {
        let (db_len, db_mtime) = db_fingerprint(db_path).ok_or("db stat")?;
        let mem = Self::rebuild_index(&self.db)?;
        let bytes = crate::mmap_index::build_from_memory_index(&mem, db_len, db_mtime)?;
        let path = mmap_path_for(db_path);
        let af = atomicwrites::AtomicFile::new(&path, atomicwrites::OverwriteBehavior::AllowOverwrite);
        af.write(|f| std::io::Write::write_all(f, &bytes))
            .map_err(|e| e.to_string())?;
        Ok(bytes.len())
    }

    /// **Android 端专用**：加载 mmap 内存索引（`<db>.midx`），让候选查询变成 µs 级。    ///
    /// 与 `load_or_build_index` 的区别：后者在 2026-09-02 因"残页/跳页/回删卡顿"被整体停用
    /// （Windows 端至今仍走 SQLite）。Android 端的诉求相反 —— 手机上词库查询必须 µs 级，
    /// 否则打字会"卡手"。所以做成**独立入口**，不影响 Windows/Linux 的既有行为。
    ///
    /// 流程：只加载已有的 `<db>.midx`（映射即用，微秒级、零解析、多进程共享物理页）；
    /// **不在设备上重建**（重建要几十秒 + 上百 MB 内存，手机扛不住）——
    /// `.midx` 由 PC 侧 `midxtool` 生成、随词库一起下发。
    /// 没有 `.midx` 或指纹对不上 → 返回 false，调用方继续走 SQLite（功能不受影响）。
    pub fn load_mmap_index(&mut self, db_path: &str) -> (bool, &'static str) {
        match self.try_load_mmap(db_path) {
            Some(mem) => {
                self.mem = Some(mem);
                (true, "mmap")
            }
            None => (false, "sqlite"),
        }
    }

    /// 尝试 mmap 加载;文件缺失/损坏/词库已变 返回 None。
    fn try_load_mmap(&self, db_path: &str) -> Option<MemoryIndex> {
        let path = mmap_path_for(db_path);
        if !path.exists() {
            return None;
        }
        let (db_len, db_mtime) = db_fingerprint(db_path)?;
        let m = crate::mmap_index::MmapIndex::map(&path, db_len, db_mtime).ok()?;
        Some(MemoryIndex::from_mmap(m))
    }

    /// 原子写 mmap 缓存(先写临时文件再 rename)。
    fn save_mmap(db_path: &str, mem: &MemoryIndex) -> Result<(), String> {
        let path = mmap_path_for(db_path);
        let (db_len, db_mtime) = db_fingerprint(db_path).ok_or("db stat")?;
        let bytes = crate::mmap_index::build_from_memory_index(mem, db_len, db_mtime)?;
        let af = atomicwrites::AtomicFile::new(&path, atomicwrites::OverwriteBehavior::AllowOverwrite);
        af.write(|f| std::io::Write::write_all(f, &bytes))
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 尝试从缓存加载内存索引;缓存缺失/损坏/词库已变 返回 None。
    fn try_load_cached(&self, db_path: &str) -> Option<MemoryIndex> {
        let cache = cache_path_for(db_path);
        let bytes = std::fs::read(&cache).ok()?;
        let (db_len, db_mtime) = db_fingerprint(db_path)?;
        // 头部定长解码(bincode 对定长结构是定长的),先解析头部校验再解 body。
        let header_len = bincode::serialized_size(&IndexHeader {
            magic: *b"PIXC",
            version: 0,
            db_len: 0,
            db_mtime_secs: 0,
        })
        .ok()? as usize;
        if bytes.len() < header_len {
            return None;
        }
        let header: IndexHeader = bincode::deserialize(&bytes[..header_len]).ok()?;
        if header.magic != *b"PIXC"
            || header.version != INDEX_CACHE_VERSION
            || header.db_len != db_len
            || header.db_mtime_secs != db_mtime
        {
            return None; // 词库变了或版本不符 → 触发重建
        }
        MemoryIndex::from_bytes(&bytes[header_len..]).ok()
    }

    /// 原子写缓存(先写临时文件再 rename,避免中途崩溃留半个坏文件)。
    fn save_cached(db_path: &str, mem: &MemoryIndex) -> Result<(), String> {
        let cache = cache_path_for(db_path);
        let (db_len, db_mtime) = db_fingerprint(db_path).ok_or("db stat")?;
        let header = IndexHeader {
            magic: *b"PIXC",
            version: INDEX_CACHE_VERSION,
            db_len,
            db_mtime_secs: db_mtime,
        };
        let mut bytes = bincode::serialize(&header).map_err(|e| e.to_string())?;
        bytes.extend_from_slice(&mem.to_bytes()?);
        let af = atomicwrites::AtomicFile::new(&cache, atomicwrites::OverwriteBehavior::AllowOverwrite);
        af.write(|f| std::io::Write::write_all(f, &bytes))
            .map_err(|e| format!("atomic write: {e}"))
    }

    pub fn set_fuzzy_rules(&mut self, rules: Vec<String>) {
        self.fuzzy_rules = rules;
    }

    pub fn segment(&self, input: &str) -> Vec<String> {
        self.syll.segment(input)
    }

    fn is_full_pinyin(&self, s: &str) -> bool {
        self.syll.is_full_pinyin(s)
    }

    /// P0 中英文自动判定(2026-11-16)。
    pub fn classify_input(&self, input: &str) -> crate::lang::LangKind {
        crate::lang::decide(&self.syll, input, self.has_exact_hit(input))
    }

    /// P0 中英文自动判定(调用方已知是否有精确命中时用)。
    pub fn classify_input_with(&self, input: &str, has_exact_hit: bool) -> crate::lang::LangKind {
        crate::lang::decide(&self.syll, input, has_exact_hit)
    }

    /// 词库里是否有 `input` 的**精确**命中(整词 key 或混拼 jp 完全相等)。
    ///
    /// 这是中英判定的关键信号。不能用 `query()` 的候选数代替 —— query 带模糊音
    /// 与贪心切分兼底,`hello`/`world` 这类纯英文实测都能返回 50 条候选。
    ///
    /// **2026-11-25 补混拼**:`limancx` / `yakebicx` 这类「全拼头 + 尾部首字母」
    /// 以前这里查不到精确命中 → lang 判英文 → 候选被替换成原样字母 → 用户看到
    /// 「打了混拼没候选,直接上屏字母」。现在把混拼通道也算作拼音命中:
    ///   - 主词库:key 前缀 + 尾部首字母匹配(与 query() 完全相同的一套判定);
    ///   - 用户手工造词:`--add-word` 写的简拼别名 / 混合形态。
    /// 纯全拼早就被 lang 直接判拼音,这里提前返回省掉几次查询。
    fn has_exact_hit(&self, input: &str) -> bool {
        if input.is_empty() {
            return false;
        }
        if self
            .db
            .query_phrase(input, 1)
            .map(|r| !r.is_empty())
            .unwrap_or(false)
        {
            return true;
        }
        if self
            .db
            .query_phrase_by_jp(input, 1)
            .map(|r| !r.is_empty())
            .unwrap_or(false)
        {
            return true;
        }
        if self.is_full_pinyin(input) {
            return false; // 全拼音节输入恒判拼音,不必再走混拼通道
        }
        if !self.query_phrase_abbrev_tail(input).is_empty() {
            return true;
        }
        !self.db.manual_words_mixed(input).is_empty()
    }

    /// P0 智能标点(2026-11-16)。
    /// `reset=true` 先归位引号状态(换输入框/上屏/回车时调用)。
    /// 返回 None 表示不映射,调用方按原键处理。
    pub fn punct_map(&self, ch: char, reset: bool) -> Option<String> {
        let mut st = self.punct_state.lock().unwrap_or_else(|e| e.into_inner());
        if reset {
            st.reset();
        }
        crate::punct::map(ch, &mut st).map(|s| s.to_string())
    }

    /// 单独归位标点状态(不回传字符)。
    pub fn punct_reset(&self) {
        let mut st = self.punct_state.lock().unwrap_or_else(|e| e.into_inner());
        st.reset();
    }

    /// P2 简繁:设置输出模式。
    pub fn set_script(&self, mode: ScriptMode) {
        self.script_mode.store(mode.as_i32() as u8, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn script(&self) -> ScriptMode {
        ScriptMode::from_i32(self.script_mode.load(std::sync::atomic::Ordering::Relaxed) as i32)
    }

    /// 循环切换到下一种模式（UI 按钮用）。
    pub fn cycle_script(&self) -> ScriptMode {
        let next = self.script().next();
        self.set_script(next);
        next
    }

    /// 兼容旧接口：true = 简→繁，false = 原样。
    pub fn set_trad(&self, on: bool) {
        self.set_script(if on { ScriptMode::S2T } else { ScriptMode::Off });
    }

    pub fn trad_enabled(&self) -> bool {
        self.script() == ScriptMode::S2T
    }

    /// P2 简繁:上屏前转换。原样模式下直接返回（调用方无需自己判断开关）。
    pub fn to_script(&self, text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }
        match self.script() {
            ScriptMode::Off => text.to_string(),
            ScriptMode::S2T => {
                if !crate::s2t::s2t_may_change(text) {
                    text.to_string()
                } else {
                    crate::s2t::to_traditional(text)
                }
            }
            ScriptMode::T2S => {
                if !crate::s2t::t2s_may_change(text) {
                    text.to_string()
                } else {
                    crate::s2t::to_simplified(text)
                }
            }
        }
    }

    /// 兼容旧接口：当前模式下的转换（原名 to_trad）。
    pub fn to_trad(&self, text: &str) -> String {
        self.to_script(text)
    }

    /// 九宫格（T9）候选：数字串 → 候选（走 db 的 t9 表）。黑白名单照旧生效。
    pub fn query_t9(&self, digits: &str) -> Vec<Candidate> {
        if digits.is_empty() {
            return Vec::new();
        }
        let rows = self.db.query_t9(digits, 50).unwrap_or_default();
        let seen: HashMap<String, i64> = rows.into_iter().collect();
        let filtered = self.apply_blacklist_whitelist(seen);
        let mut out: Vec<Candidate> = filtered.into_iter().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out.truncate(50);
        out
    }

    /// **精确拼音查询**：只查 key 完全等于该拼音的词，**不做**模糊音扩展、**不做**前缀扩展。
    ///
    /// key 用词库里的写法（不带空格）："ni" / "nihao"。
    /// 九宫格里用它把候选收敛到用户选定的那个读音 —— 这是"选读音"能真正生效的关键。
    pub fn query_pinyin_exact(&self, key: &str) -> Vec<Candidate> {
        let key = key.trim();
        if key.is_empty() {
            return Vec::new();
        }
        let mut seen: HashMap<String, i64> = HashMap::new();
        // 词组（phrase 表 key 不带撇号，如 "nihao"）
        for (w, wt) in self.db.query_phrase(key, 30).unwrap_or_default() {
            upsert(&mut seen, w, wt);
        }
        // 单字（pinyin 表，key 就是音节，如 "ni" → 你/泥/尼…）
        for (w, wt) in self.db.query_pinyin(key, 30).unwrap_or_default() {
            upsert(&mut seen, w, wt);
        }
        let filtered = self.apply_blacklist_whitelist(seen);
        let mut out: Vec<Candidate> = filtered.into_iter().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out.truncate(50);
        out
    }

    /// 某读音的**精确段**（拼音恰好等于该读音的词）：2 条 SQL。
    /// 合并视图里每个读音都只查这一段（够用，且便宜）。
    fn query_reading_exact(&self, key: &str) -> Vec<Candidate> {
        let key = key.trim();
        if key.is_empty() {
            return Vec::new();
        }
        let mut seen: HashMap<String, i64> = HashMap::new();
        if let Some(mem) = &self.mem {
            // 内存索引(**精确桶**):key 恰好等于该读音的词 —— 与 SQLite `WHERE key=?` 同语义。
            // 多字词通道(对齐 query_phrase)+ 单字通道(对齐 query_pinyin),各自 LIMIT 20。
            //
            // 不能用前缀桶(query_prefix):那是**前缀累积**的,查 "ni" 会把 "niao"/"nie" 的词
            // 也带进来,与 SQLite 精确匹配结果不一致 → 三端候选不同。
            for (w, wt) in mem.query_own(key).into_iter().take(20) {
                upsert(&mut seen, w, wt);
            }
            for (w, wt) in mem.query_own_single(key).into_iter().take(20) {
                upsert(&mut seen, w, wt);
            }
        } else {
            for (w, wt) in self.db.query_phrase(key, 20).unwrap_or_default() {
                upsert(&mut seen, w, wt);
            }
            for (w, wt) in self.db.query_pinyin(key, 20).unwrap_or_default() {
                upsert(&mut seen, w, wt);
            }
        }
        let filtered = self.apply_blacklist_whitelist(seen);
        let mut out: Vec<Candidate> = filtered.into_iter().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out
    }

    /// 某读音下的候选，分两段：
    ///   exact  —— 拼音**恰好等于**该读音的词（"ni" → 你/泥/尼…；"nihao" → 你好）
    ///   prefix —— 拼音以该读音**开头**的更长词（"ni" → 你们/年/鸟…）
    ///
    /// 两段都走 phrase/pinyin 表（phrase.key 上有 idx_phrase_key，前缀范围走索引，~1ms），
    /// 不走 t9 表的 range 扫描（那个要排序整段，几十~几百 ms）。
    fn query_reading(&self, key: &str) -> (Vec<Candidate>, Vec<Candidate>) {
        let exact = self.query_reading_exact(key);
        let mut prefix: HashMap<String, i64> = HashMap::new();
        if let Some(mem) = &self.mem {
            // 前缀段：SQLite 走 `query_phrase_by_key_prefix`（key 前缀范围 + 权重降序 LIMIT 20），
            // 内存侧用**前缀桶**(同一语义：key 以该读音开头；单字走 top1 桶不进这里)。
            // 单字母前缀范围太大 → 与 SQLite 一样跳过(len < 2 不查)。
            if key.trim().len() >= 2 {
                for (w, wt) in mem.query_prefix(key.trim()) {
                    upsert(&mut prefix, w, wt);
                }
            }
        } else if key.trim().len() >= 2 {
            // 前缀段：只在读音 ≥2 个字母时做（单字母前缀范围太大，会拖慢）
            for (w, wt) in self.db.query_phrase_by_key_prefix(key.trim(), 20).unwrap_or_default() {
                upsert(&mut prefix, w, wt);
            }
        }
        // exact 里已有的词不再重复出现在前缀段
        for c in &exact {
            prefix.remove(&c.0);
        }
        let prefix = self.apply_blacklist_whitelist(prefix);
        let mut p: Vec<Candidate> = prefix.into_iter().collect();
        p.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        (exact, p)
    }

    /// 九宫格：用户**选定某个读音**后的候选（该读音的精确词 + 以它开头的更长词）。
    pub fn query_reading_candidates(&self, reading: &str) -> Vec<Candidate> {
        // 读音里的空格/分词符只是给人看的，词库 key 只要字母
        let key: String = reading.chars().filter(|c| c.is_ascii_alphabetic()).collect();
        let (e, p) = self.query_reading(&key);
        let mut out = e;
        out.extend(p);
        // 学成词置顶（2026-11-19 补）：九宫格走的是读音查询路径，原来**完全没并 user_word**
        // → 用户在 26 键学过的词，切到九宫格就"消失"了（学过的词只在 26 键置顶）。
        // 语义与 26 键 query() 对齐：学成词位置锁定（固定权重 + 学习次序）。
        // key 用**前缀**匹配：学习记的是词的完整读音，读音只是它的前缀（见 t9_query 的说明）。
        let mut learned: Vec<Candidate> = Vec::new();
        for (w, _seq) in self.db.user_words_with_prefix(&key) {
            if self.db.is_blacklisted(&w) {
                continue;
            }
            if !out.iter().any(|c| c.0 == w) {
                learned.push((w, crate::db::LEARN_PIN_WEIGHT));
            }
        }
        let mut res = learned;
        res.extend(out);
        res.truncate(50);
        res
    }

    /// **26 键**：把用户输入解析成"读音"，给候选栏第一行显示（简拼的**可发现性**）。
    ///
    /// 背景（2026-11-19 用户反馈"九宫格能一次出四个字，26 键能不能"）：
    /// 26 键**本来就能**一次出多字词 —— 词库 `phrase.jp` 存的是"每音节首字母"的简拼
    /// （"你好旧时光" → `nhjsg`），所以 `nhjsg` 5 键就能出 5 字词，按键次数与九宫格持平。
    /// 但界面第一行只显示用户敲的字母，用户根本看不出"这串字母被解读成了 5 个音节"，
    /// 于是这个能力**等于不存在**。九宫格没这问题（第一行直接显示拼音）。
    ///
    /// 返回 `Some((词, 带空格的读音))`；解析不出来返回 `None`（调用方继续显示原输入）。
    ///
    /// ⚠️ 注意：这个"按输入反查"的结果**可能和候选排序不一致**（见 `pinyin_of_word` 的说明），
    /// 所以界面应当优先用 `pinyin_of_word(首候选)`，这里只当兜底。
    pub fn resolve_pinyin(&self, input: &str) -> Option<(String, String)> {
        let input = input.trim();
        if input.is_empty() {
            return None;
        }
        // 1) 全拼：直接切分显示（"nihao" → "ni hao"）。要求**恰好**切完（不回退贪心），
        //    否则简拼串会被切成碎片（如 "nhjsg" → "n/h/j/s/g"）显示给用户，纯噪声。
        if let Some(segs) = self.syll.segments(input) {
            if segs.len() >= 2 {
                return Some((input.to_string(), segs.join(" ")));
            }
            return None;
        }
        // 2) 简拼/混拼：jp 精确命中 → 取词库里的全拼 key → 切分成音节
        let rows = self.db.query_phrase_jp_reading(input, 1).ok()?;
        let (key, word, _wt) = rows.into_iter().next()?;
        let segs = self.syll.segment(&key);
        if segs.is_empty() {
            return None;
        }
        Some((word, segs.join(" ")))
    }

    /// **26 键**：由**候选词**反查它的读音（"你好" → "ni hao"），给候选栏第一行显示。
    ///
    /// 为什么必须按词反查、而不是按输入反查（`resolve_pinyin`）：
    /// 候选的排序规则是"精确命中优先 + 多字优先 + 权重"，而 `phrase.jp` 的命中是"纯权重"。
    /// 两者对同一串输入经常给出**不同的首选**（实测 "nhxf"：jp 最高权重是「南湖蟹粉」，
    /// 候选首选却是「你好幸福」）→ 显示的读音与首候选对不上，用户会懵。
    /// 按词反查则保证"第一行读音 = 首候选的读音"，永远自洽。
    pub fn pinyin_of_word(&self, word: &str) -> Option<String> {
        let key = self.db.query_phrase_key_for_word(word)?;
        let segs = self.syll.segment(&key);
        if segs.is_empty() {
            None
        } else {
            Some(segs.join(" "))
        }
    }

    /// 九宫格：数字串 → 读音（**不查库**，纯内存，微秒级）。
    ///
    /// 用途：Android 主线程在按键后**立刻**把首行显示成拼音（"ni"）而不是数字串，
    /// 不等候选查询返回 —— 否则九宫格首行要空等几十~上百毫秒，手感明显比 26 键"卡"。
    pub fn t9_readings(&self, digits: &str) -> Vec<String> {
        self.t9_reading_pairs(digits)
            .into_iter()
            .map(|(_, p)| p)
            .collect()
    }

    /// 展开 + 按**词库 key** 去重（"ni hao" 与 "ni ha o" 都是 "nihao"，留音节少的）。
    /// 返回 (词库key, 读音)。
    fn t9_reading_pairs(&self, digits: &str) -> Vec<(String, String)> {
        let mut uniq: Vec<(String, String)> = Vec::new();
        let mut seen_key: HashMap<String, usize> = HashMap::new();
        for p in self.syll.t9_expand(digits) {
            // 词库 key 不带空格/分词符："ni hao" / "xi'an" → "nihao" / "xian"
            let key: String = p.chars().filter(|c| c.is_ascii_alphabetic()).collect();
            match seen_key.get(&key) {
                Some(&idx) => {
                    if p.split(' ').count() < uniq[idx].1.split(' ').count() {
                        uniq[idx].1 = p;
                    }
                }
                None => {
                    seen_key.insert(key.clone(), uniq.len());
                    uniq.push((key, p));
                }
            }
        }
        uniq
    }

    /// **九宫格**：数字串 →（可能的读音，候选词）。
    ///
    /// 做法：先用标准音节表把数字串**精确**展开成若干种读音
    /// （"64" → mi / ni / oi），再逐个读音去词库取候选。
    ///
    /// 这样界面能同时拿到：
    ///   · 候选词（先精确命中、再前缀预测，各自按权重降序）
    ///   · 每种读音（用户点一下就把候选收敛到那个读音）
    ///
    /// 数字串拼不出标准音节时（"9"、只按了一键等），读音为空 —— 界面据此
    /// 不显示读音消歧条（不装作知道读音），候选退回 t9 表的整串命中。
    pub fn t9_query(&self, digits: &str) -> T9Result {
        if digits.is_empty() {
            return T9Result::default();
        }
        // 展开出的读音可能指向同一个词库 key（"ni hao" 与 "ni ha o" 都是 "nihao"），
        // 按 key 去重，保留**音节更少**（更自然）的那一种。
        let uniq = self.t9_reading_pairs(digits);

        // 每个读音 → (精确段, 前缀段)。读音条里的每个读音都要有自己的候选，
        // 所以这里对每个读音都查两段（词库在**内部存储**时整轮只要几毫秒）。
        // 元组：(词库key, 读音, 精确段, 前缀段)
        let mut entries: Vec<(String, String, Vec<Candidate>, Vec<Candidate>)> = Vec::new();
        for (key, p) in &uniq {
            let (e, pre) = self.query_reading(key);
            if e.is_empty() && pre.is_empty() {
                continue;
            }
            entries.push((key.clone(), p.clone(), e, pre));
        }
        if entries.is_empty() {
            // 拼不出读音 → 只能给词，界面不显示读音条。
            // 分词符 `'` 不是 t9 表键的一部分，回退查库时先剥掉。
            let bare: String = digits.chars().filter(|c| c.is_ascii_digit()).collect();
            return T9Result {
                pinyins: Vec::new(),
                words: self.db
                    .query_t9(&bare, 30)
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
            };
        }
        // 常用读音在前：按该读音下最高权重的候选排；同权重时音节少的在前
        entries.sort_by(|a, b| {
            let ka = a.2.first().map(|c| c.1).unwrap_or(0);
            let kb = b.2.first().map(|c| c.1).unwrap_or(0);
            kb.cmp(&ka).then(a.3.len().cmp(&b.3.len())).then(a.1.cmp(&b.1))
        });
        let pinyins: Vec<String> = entries.iter().map(|e| e.1.clone()).collect();

        // 合并：先所有精确段（按权重），再所有前缀段（按权重）
        let mut seen: HashMap<String, i64> = HashMap::new();
        for (_, _, e, _) in &entries {
            for (w, wt) in e {
                upsert(&mut seen, w.clone(), *wt);
            }
        }
        let mut words: Vec<Candidate> = seen.into_iter().collect();
        words.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut have: HashSet<String> = words.iter().map(|c| c.0.clone()).collect();
        let mut extra: Vec<Candidate> = Vec::new();
        for (_, _, _, pre) in &entries {
            for (w, wt) in pre {
                if have.insert(w.clone()) {
                    extra.push((w.clone(), *wt));
                }
            }
        }
        extra.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        words.extend(extra);
        // 学成词置顶（2026-11-19 补，handoff 下一步 #3）：九宫格原来**完全没并 user_word**
        // → 刚在 26 键学过的词，切到九宫格就不见了，用户会以为"九宫格不准"。
        // 语义与 26 键 query() 一致：学成词固定权重 + 按学习次序（seq 升序）排最前。
        //
        // key 用**前缀**匹配（2026-11-20 真机实测后修正）：学习时记的是**词的完整读音**
        // （"米高扬" → "migaoyang"），而九宫格按**读音**查（"mi gao" → "migao"）——
        // 精确匹配查不到，学成词就不置顶。读音是词读音的**前缀**，所以按前缀查。
        //
        // ⚠️ 只合并**首行读音**（entries[0]，按候选权重排在第一的那个）的学成词！
        // 2026-11-20 用户报"打 nihao 出来米高扬"：原来遍历**所有**读音（ni hao / mi gao / …），
        // "mi gao" 的前缀匹配把学过的「米高扬」也置顶了 —— 用户还没选读音，
        // 不相干读音的学成词就跑到第一屏，而且和「你好」同权重、按学习顺序排在前面。
        // 用户明确点了某个读音后，`query_reading_candidates` 会按那个读音查，不受这里影响。
        let mut learned: Vec<Candidate> = Vec::new();
        let mut learned_seen: HashSet<String> = HashSet::new();
        if let Some((first_key, _p, _e, _pre)) = entries.first() {
            for (w, _seq) in self.db.user_words_with_prefix(first_key) {
                if self.db.is_blacklisted(&w) {
                    continue;
                }
                if learned_seen.insert(w.clone()) {
                    learned.push((w, crate::db::LEARN_PIN_WEIGHT));
                }
            }
        }
        let mut out: Vec<Candidate> = Vec::with_capacity(learned.len() + words.len());
        for (w, wt) in learned {
            let lw = w.clone();
            words.retain(|c| c.0 != lw);
            out.push((w, wt));
        }
        out.extend(words);
        out.truncate(50);
        T9Result { pinyins, words: out }
    }

    /// **诊断用**:同一输入分别走 mmap 路径与 SQLite 路径,给出耗时与首屏候选对比。
    ///
    /// Android 端要走 mmap,但必须证明"结果与 SQLite 一致"——否则三端候选不同、
    /// 用户会发现"同样的词在手机和电脑不一样"。做法:临时把内存索引摘掉查一次 SQLite,
    /// 再装回去查一次 mmap,输出 JSON 供上层比对(首屏 10 个候选是否逐条相同)。
    ///
    /// 注意:这是**诊断**接口,会短暂切换引擎状态,不要在正常打字路径里调用。
    pub fn query_profile(&mut self, input: &str) -> String {
        // 1) SQLite 路径(临时摘掉内存索引)
        let mem_backup = self.mem.take();
        let t = std::time::Instant::now();
        let sql = self.query(input);
        let sql_us = t.elapsed().as_micros() as u64;
        // 2) mmap 路径(装回内存索引)
        self.mem = mem_backup;
        let t = std::time::Instant::now();
        let mm = self.query(input);
        let mem_us = t.elapsed().as_micros() as u64;
        // 3) 首屏 10 个候选是否逐条一致
        let head = |v: &Vec<Candidate>| -> Vec<String> {
            v.iter().take(10).map(|(w, _)| w.clone()).collect()
        };
        let (h1, h2) = (head(&sql), head(&mm));
        serde_json::json!({
            "input": input,
            "sql_us": sql_us,
            "mem_us": mem_us,
            "sql_n": sql.len(),
            "mem_n": mm.len(),
            "same_head": h1 == h2,
            "sql_top": h1,
            "mem_top": h2,
        })
        .to_string()
    }

    /// 五笔候选查询(wubi86 表,精确编码匹配,权重降序)。
    /// 与拼音 `query` 并列,供 FFI `prisir_ime_query_wubi` 调用。
    pub fn query_wubi(&self, input: &str) -> Vec<Candidate> {
        let key = input.trim().to_lowercase();
        if key.is_empty() {
            return Vec::new();
        }
        self.db.query_wubi(&key, 50).unwrap_or_default()
    }

    /// 拼音候选查询(对齐 Python _query_pinyin)
    pub fn query(&self, input: &str) -> Vec<Candidate> {
        // 单字母(2026-09-02):直接走前缀高权单字桶,先于一切合并逻辑返回。
        // 对齐外挂内存路径 trie z→在/张/中/这/再…(前缀节点高权桶)。
        // 放在最前是因为「z」会被 query_phrase_by_key_prefix 命中 phrase 表 z 开头的词,
        // 使 seen 非空而提前返回窄结果;外挂语义是单字母就给前缀高权字,不做整词/混拼。
        // 新增:单字母也作为拼音别名查询(如打 "s" 命中 "世界"的别名 "s")。
        if input.chars().count() == 1 {
            let mut seen: HashMap<String, i64> = HashMap::new();
            if let Some(mem) = &self.mem {
                // 1) 前缀高权单字(对齐 query_pinyin_prefix_top):两者都是**权重降序**取前 N,
                //    所以首屏候选必然一致(mem 桶 24 条 vs SQL 50 条,只差能翻的页数)。
                for (word, w) in mem.query_prefix_single(input) {
                    upsert(&mut seen, word, w);
                }
                // 2) 拼音别名:phrase 表中 key == 单字母 的词组(如 key="s" value="世界")
                for (word, w) in mem.query_own(input).into_iter().take(50) {
                    upsert(&mut seen, word, w);
                }
            } else {
                // 1) 前缀高权单字
                if let Ok(rows) = self.db.query_pinyin_prefix_top(input, 50) {
                    for (word, w) in rows {
                        upsert(&mut seen, word, w);
                    }
                }
                // 2) 拼音别名:phrase 表中 key == 单字母 的词组(如 key="s" value="世界")
                if let Ok(rows) = self.db.query_phrase(input, 50) {
                    for (word, w) in rows {
                        upsert(&mut seen, word, w);
                    }
                }
            }
            // 黑白名单过滤+加权
            let filtered = self.apply_blacklist_whitelist(seen);
            // 2026-11-19:原来直接 `into_iter().collect()` —— HashMap 迭代序是**随机**的,
            // 同一输入每次候选顺序都不同(用户看到"候选在跳")。补确定性排序
            // (权重降序 + 字典序决胜),与主分支的决胜键保持一致。
            let mut out: Vec<Candidate> = filtered.into_iter().collect();
            out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            return out;
        }
        let n_syll = self.segment(input).len().max(1);
        let mut seen: HashMap<String, i64> = HashMap::new();
        // 模糊音扩展出来的词:排序最低,但**要给它们留出可见位置**。
        // 2026-11-17:原来 truncate(50) 会把它们整体切掉(打 zi 一个翘舌字都看不到),
        // 相当于"开了模糊音也白开"。现在留 10 个槽位(第 41~50 名,多翻两页能到)。
        let mut fuzzy_words: std::collections::HashSet<String> = std::collections::HashSet::new();

        // 模糊音扩展(仅单音节单字查询)
        let fuzzy_inputs = if n_syll == 1 { self.fuzzy_expand(input) } else { vec![input.to_string()] };

        if let Some(mem) = &self.mem {
            // 内存索引(**精确桶**):逐条对齐 SQLite 分支的三个通道与 LIMIT。
            //   1) 原音单字   ← query_pinyin(orig, 50)
            //   2) 模糊音单字 ← query_pinyin(fp, 10) + 权重降档 FUZZY_DEMOTE
            //   3) 整词词组   ← query_phrase(input, 50) + 字数==音节数过滤
            //
            // 用精确桶(query_own)而不是前缀桶(query_prefix):前缀桶是**累积**的,
            // 查 "ni" 会带出 "niao"/"nie" 的词,与 SQLite `WHERE key=?` 不一致。
            let orig = input.to_string();
            for (word, w) in mem.query_own_single(&orig).into_iter().take(50) {
                upsert(&mut seen, word, w);
            }
            for fp in fuzzy_inputs.iter().filter(|fp| **fp != orig) {
                for (word, w) in mem.query_own_single(fp).into_iter().take(10) {
                    // 模糊字权重压到最低档之下,确保不反超原音字(保留相对顺序)
                    fuzzy_words.insert(word.clone());
                    upsert(&mut seen, word, w - FUZZY_DEMOTE);
                }
            }
            for (word, w) in mem.query_own(&orig).into_iter().take(50) {
                if word.chars().count() != n_syll {
                    continue;
                }
                upsert(&mut seen, word, w);
            }
        } else {
            // 2026-09-02 候选覆盖/排序修复(对齐外挂内存路径语义):
            // 原音(input 本身)与模糊扩展音(zhi)分开排序——原音字按权重排前,模糊字殿后,
            // 不全局混排。外挂内存路径之所以 zi 出「自/字/子/资…」且含耔/缁/赀,正是因为
            // zi 原音字在 zi 节点单字桶里天然排在模糊 zhi 字之前;混排会让高权 zhi 字
            // 反超低频 zi 原音字(耔 被挤出前页)。这里复刻该语义。
            // fuzzy_expand 保证原音在前,fuzzy_inputs[0]==input,其余为模糊扩展。
            let orig = input.to_string();
            // 1) 原音字:按权重降序,先占满
            if let Ok(rows) = self.db.query_pinyin(&orig, 50) {
                for (word, w) in rows {
                    upsert(&mut seen, word, w);
                }
            }
            // 2) 模糊扩展音字:殿后(仅当模糊音开启且与原音不同才查)
            for fp in fuzzy_inputs.iter().filter(|fp| **fp != orig) {
                // 模糊音只取 10 条:候选总数 truncate(50),若模糊字也取 50,
                // 原音字会把它们全挤出前 50(实测 zi 的 之/知/只 一个都进不来)。
                if let Ok(rows) = self.db.query_pinyin(fp, 10) {
                    for (word, w) in rows {
                        // 模糊字权重压到最低档之下,确保不反超原音字(保留相对顺序)
                        fuzzy_words.insert(word.clone());
                        upsert(&mut seen, word, w - FUZZY_DEMOTE);
                    }
                }
            }
            // 3) 整词词组(错配过滤)
            if let Ok(rows) = self.db.query_phrase(input, 50) {
                for (word, w) in rows {
                    if word.chars().count() != n_syll {
                        continue;
                    }
                    upsert(&mut seen, word, w);
                }
            }
        }

        // 混拼词组(正反向)
        for (word, w) in self.query_phrase_mixed(input) {
            upsert(&mut seen, word, w);
        }

        // 精确命中优先(2026-11-17):打完整拼音时,「那条词本身」必须排第一。
        //
        // 症状:query("duomotai") 出「多模态模型/推理/基准…」,而「多模态」排到第 8。
        // 根因:排序只有 (多字优先, 权重降序, 字典序),没有"这条候选的 key/jp 恰好等于输入"这一档;
        //       而基础库权重量级混乱(多模态=100, 它的衍生词=19050),整词被自己的衍生词挤下去。
        // 做法:单独查 4 个精确通道(key= / jp= × phrase/pinyin),命中的词
        //       ① 保证进 seen(防止上游 truncate 把它截掉) ② 排序时排在权重之前。
        // 不改任何权重语义,也不动 fuzzy/whitelist 的既有档位。
        let exact = self.exact_hit_words(input);
        for (word, w) in &exact {
            upsert(&mut seen, word.clone(), *w);
        }
        // 2026-11-21: 全拼 key 精确 vs 简拼 jp 精确分开。
        // 单音节输入 de 时，jp='de' 的「单个」(本应 dg) 会与「的」同级 exact，
        // 再被「多字>单字」顶到第一；key 精确必须高于 jp 精确。
        let key_exact_set: HashSet<String> = self.key_exact_words(input);
        let exact_set: HashSet<&str> = exact.iter().map(|(w, _)| w.as_str()).collect();

        // 黑白名单过滤+白名单加权(对齐 Python professional_corrector):
        // 1. 黑名单词彻底剔除
        // 2. 白名单词给 +WHITELIST_BONUS 权重,同等权重下排更前
        seen = self.apply_blacklist_whitelist(seen);

        // 学习权重合并(2026-09-03 引入,2026-09-04 改学成置顶+位置锁定):
        // user.db 里用户学过的词提到最前。学成词给固定权重 LEARN_PIN_WEIGHT(>词库 max≈26000),
        // **不再叠加 base+lw、不再 +1 累积、不做时间衰减** → 学成词一旦学会位置永久锁定,
        // 解决「偶尔用的词位置老变、无法固化调用」(用户 2026-09-04 明确不要衰减)。
        // 多个学成词按 user_words_for 返回的 seq(学习先后)升序稳定排序,先学在前。
        // 学习库独有的新词(主库没有的自造词)也补进来。user.db 只读查询,微秒级。
        // seen 是 HashMap<String, i64>,为保住学成词的 seq 次序,单独收集学成词再前置。
        let user_words = self.db.user_words_for(input);
        // 学成词也过滤黑名单
        let user_words: Vec<(String, i64)> = user_words
            .into_iter()
            .filter(|(w, _)| !self.db.is_blacklisted(w))
            .map(|(w, s)| (w, s))
            .collect();
        if !user_words.is_empty() {
            // 学成词从 seen 移除(若在),稍后按 seq 序前置,避免被 HashMap 随机序打乱。
            for (word, _seq) in &user_words {
                seen.remove(word);
            }
        }

        // 手工造词(2026-11-24 快捷造词):位置由 pos 决定,在候选组装完成后插位。
        // 与自动学成词(seq>0,前置置顶)是两条互不干扰的路径。
        //
        // 精确 key 查不到时再试「混拼」(2026-11-25):手工词存的是全拼主行,
        // 用户打 yakebicx 这种前全拼后简拼的输入时 key 对不上,靠音节对齐兜住。
        let mut manual = self.db.manual_words_for(input);
        if manual.is_empty() {
            manual = self.db.manual_words_mixed(input);
        }

        if !seen.is_empty() || !user_words.is_empty() || !manual.is_empty() {
            // 多字真整词优先于单字,同权重整词靠前
            let mut merged: Vec<Candidate> = seen.into_iter().collect();
            // 排序必须完全确定(2026-09-02 修「翻页候选顺序漂移」):seen 是 HashMap,
            // into_iter 顺序每次随机;若 sort 只比多字/权重,同权重单字的相对顺序由
            // HashMap 随机顺序决定 → 每次 query 候选顺序都不同 → 翻页页码/内容漂移。
            // 加决胜键:权重相同再比字符串本身(字典序),保证同输入必得同序。
            merged.sort_by(|a, b| {
                let a_multi = a.0.chars().count() > 1;
                let b_multi = b.0.chars().count() > 1;
                let a_key = key_exact_set.contains(a.0.as_str());
                let b_key = key_exact_set.contains(b.0.as_str());
                let a_exact = exact_set.contains(a.0.as_str());
                let b_exact = exact_set.contains(b.0.as_str());
                // 全拼精确 > 简拼/其它精确 > 多字整词 > 权重 > 字典序
                b_key
                    .cmp(&a_key)
                    .then(b_exact.cmp(&a_exact))
                    .then(b_multi.cmp(&a_multi))
                    .then(b.1.cmp(&a.1))
                    .then(a.0.cmp(&b.0))
            });
            // 学成词按 seq 升序前置到候选最前(位置锁定:固定权重 + 学习次序)。
            // user_words_for 已按 seq ASC 返回,直接前插即得稳定次序。
            let pinned: Vec<Candidate> = user_words
                .into_iter()
                .map(|(word, _seq)| (word, crate::db::LEARN_PIN_WEIGHT))
                .collect();
            let mut out = pinned;
            let (fz, rest): (Vec<Candidate>, Vec<Candidate>) = merged
                .into_iter()
                .partition(|c| fuzzy_words.contains(&c.0));
            if fz.is_empty() {
                out.extend(rest);
            } else {
                // 40 个原音 + 10 个模糊 = 50。模糊候选排在第 41~50(多翻几页能到,不需要任何开关)
                out.extend(rest.into_iter().take(40));
                out.extend(fz.into_iter().take(10));
            }
            out.truncate(50);
            let out = self.with_sentence_head(out, input, n_syll);
            return Self::splice_manual(out, &manual, 50);
        }

        // 多字母无整匹配:取首音节单字
        let segs = self.segment(input);
        if !segs.is_empty() {
            let out = self.db.query_pinyin(&segs[0], 50).unwrap_or_default();
            let out = self.with_sentence_head(out, input, n_syll);
            return Self::splice_manual(out, &manual, 50);
        }
        // 主库完全没有这条编码的候选,但用户手工造过词 → 仍然给出手工词。
        // (否则「造了词却打不出来」,是最难自查的失败模式。)
        let out = self.with_sentence_head(Vec::new(), input, n_syll);
        Self::splice_manual(out, &manual, 50)
    }

    /// 整句首选补齐(2026-11-25):保证「候选窗第一行」= 「空格上屏的那条」。
    ///
    /// 旧逻辑在**空格上屏那一刻**才用整句替换首选 → 候选窗显示的是词级首选,
    /// 上屏的却是另一个串。用户实测 `sidegeer`:候选第 1 位显示「斯德哥尔摩」,
    /// 按空格却上屏「死的戈尔」。现在把补齐提前到候选计算时,显示与上屏永远一致。
    ///
    /// 触发条件(保守,热路径几乎零开销):
    ///   - 首选字数 < 输入音节数(词库没有覆盖整串,首选只是前缀/单字兼底);
    ///   - 整句解码 字数 > 首选字数,且 ≤ 音节数(一字一音节的硬约束,挡掉乱解码);
    ///   - 首选不是用户学成/手造词(权重 ≥ LEARN_PIN_WEIGHT 时不动,用户点过的位置优先)。
    ///
    /// 于是:
    ///   - `sidegeer`(4 音节)首选「斯德哥尔摩」5 字 → 不补,空格就上屏它;
    ///   - `jintiantianqi`(4 音节)首选「今天」2 字 → 补成「今天天气」,且第一行**显示**
    ///     的就是「今天天气」,用户看得见再上屏。
    fn with_sentence_head(
        &self,
        mut out: Vec<Candidate>,
        input: &str,
        n_syll: usize,
    ) -> Vec<Candidate> {
        if n_syll < 2 || input.len() < 4 {
            return out;
        }
        let top_len = out.first().map(|c| c.0.chars().count()).unwrap_or(0);
        if let Some(top) = out.first() {
            if top.1 >= crate::db::LEARN_PIN_WEIGHT {
                return out; // 学成词/手造词位置锁定,不让整句抢
            }
        }
        if !out.is_empty() && top_len >= n_syll {
            return out; // 首选已经覆盖了整串(含前缀命中的更长词),不打扰
        }
        // 只差一个音节时不必跑整句解码:用户下一个字母就会把词补全,
        // 这里跑一次 beam search(~10ms)换来的多半是半成品句。短 2 个音节以上才补。
        if !out.is_empty() && top_len + 2 > n_syll {
            return out;
        }
        let Some(sent) = self.smart_sentence(input) else {
            return out;
        };
        let n = sent.chars().count();
        if n < 2 || n > n_syll || n <= top_len {
            return out;
        }
        if out.is_empty() {
            out.push((sent, 1));
        } else {
            out[0].0 = sent;
        }
        out
    }

    /// 把手造词按「第 N 位」插进候选序列(2026-11-24,借鉴影子输入法「快捷造词」)。
    ///
    /// 纯函数:不碰 DB、不重排已有候选,只做插位 —— 便于单测。
    /// - `pos` 是 1 起的候选位;超出长度就落在末尾(再被 cap 截)。
    /// - 同一个 pos 上有多条手工词时,按传入顺序依次占位,不会互相反序。
    /// - 词若已存在于序列(主库本来就有),先摘掉再插,避免出现两条一样的候选。
    fn splice_manual(mut out: Vec<Candidate>, manual: &[(String, i64)], cap: usize) -> Vec<Candidate> {
        if manual.is_empty() {
            out.truncate(cap);
            return out;
        }
        for (word, _) in manual {
            out.retain(|c| &c.0 != word);
        }
        // 同一 pos 已经插了几条 → 依次往后挪,保证同 pos 的顺序稳定。
        let mut last_pos = i64::MIN;
        let mut shift = 0usize;
        for (word, pos) in manual {
            let p = (*pos).max(1);
            if p != last_pos {
                last_pos = p;
                shift = 0;
            }
            let idx = ((p - 1) as usize + shift).min(out.len());
            out.insert(idx, (word.clone(), crate::db::LEARN_PIN_WEIGHT));
            shift += 1;
        }
        out.truncate(cap);
        out
    }

    /// 仅全拼 key / 单字全拼精确命中（不含 jp 简拼撞车）。
    fn key_exact_words(&self, input: &str) -> HashSet<String> {
        let mut acc: HashSet<String> = HashSet::new();
        if let Some(mem) = &self.mem {
            for (w, _) in mem.query_own(input).into_iter().take(20) {
                acc.insert(w);
            }
            for (w, _) in mem.query_own_single(input).into_iter().take(20) {
                acc.insert(w);
            }
        } else {
            if let Ok(rows) = self.db.query_phrase(input, 20) {
                for (w, _) in rows {
                    acc.insert(w);
                }
            }
            if let Ok(rows) = self.db.query_pinyin(input, 20) {
                for (w, _) in rows {
                    acc.insert(w);
                }
            }
        }
        acc
    }

    /// 精确命中集合:key 或 jp 恰好等于输入的词(多字词查 phrase,单字查 pinyin)。
    ///
    /// 4 条等值查询都走索引(毫秒以下),换来"打全了必出那个词"。
    /// 只在多字母输入时使用;单字母走前缀通道,不存在"打全"概念。
    fn exact_hit_words(&self, input: &str) -> Vec<(String, i64)> {
        let mut acc: HashMap<String, i64> = HashMap::new();
        // 有内存索引时,前三个通道直接走精确桶(µs 级),省掉 3 次 SQL;
        // pinyin 的 jp 通道没进内存索引(索引只灌了 phrase 的 jp),仍走 SQLite。
        if let Some(mem) = &self.mem {
            for (w, wt) in mem.query_own(input).into_iter().take(10) {
                upsert(&mut acc, w, wt);
            }
            for (w, wt) in mem.query_jp(input, 10) {
                upsert(&mut acc, w, wt);
            }
            for (w, wt) in mem.query_own_single(input).into_iter().take(10) {
                upsert(&mut acc, w, wt);
            }
        } else {
            // 全拼整词 / 简拼整词
            if let Ok(rows) = self.db.query_phrase(input, 10) {
                for (w, wt) in rows {
                    upsert(&mut acc, w, wt);
                }
            }
            if let Ok(rows) = self.db.query_phrase_by_jp(input, 10) {
                for (w, wt) in rows {
                    upsert(&mut acc, w, wt);
                }
            }
            // 单字全拼 / 单字简拼
            if let Ok(rows) = self.db.query_pinyin(input, 10) {
                for (w, wt) in rows {
                    upsert(&mut acc, w, wt);
                }
            }
        }
        if let Ok(rows) = self.db.query_pinyin_jp(input, 10) {
            for (w, wt) in rows {
                upsert(&mut acc, w, wt);
            }
        }
        acc.into_iter().collect()
    }

    /// 应用黑白名单过滤+白名单加权(对齐 Python professional_corrector):
    // 1. 黑名单词彻底剔除
    // 2. 白名单词给 +WHITELIST_BONUS 权重,同等权重下排更前
    fn apply_blacklist_whitelist(&self, seen: HashMap<String, i64>) -> HashMap<String, i64> {
        // 2026-11-19:原来对**每个候选词**做 2 次点查(is_blacklisted/is_whitelisted),
        // 而每个点查又查两个库(user.db ∪ 主库)→ 50 个候选 = 200 次 SQL,
        // 是单次 query 8~11ms 的真正大头(与候选桶本身快慢无关)。
        // 改成一次性读两张名单表到 HashSet —— 表本身只有几十条。
        let bl = self.db.blacklist_set();
        let wl = self.db.whitelist_set();
        let mut out = HashMap::new();
        for (word, w) in seen {
            if bl.contains(&word) {
                continue; // 黑名单:彻底剔除
            }
            let w = if wl.contains(&word) {
                w + WHITELIST_BONUS // 白名单:加权,同等权重下排更前
            } else {
                w
            };
            out.insert(word, w);
        }
        out
    }

    /// 模糊音扩展(对齐 Python _fuzzy_expand),原音在前。
    fn fuzzy_expand(&self, inp: &str) -> Vec<String> {
        let valid = |cand: &str| {
            let segs = self.segment(cand);
            segs.len() == 1 && segs[0] == cand
        };
        let mut results: Vec<String> = vec![inp.to_string()];
        for rule in self.fuzzy_rules.clone() {
            for (src, dst) in fuzzy_map(&rule) {
                if fuzzy_is_initial(&rule) {
                    if let Some(rest) = inp.strip_prefix(*src) {
                        let cand = format!("{dst}{rest}");
                        if valid(&cand) && !results.contains(&cand) {
                            results.push(cand);
                        }
                    }
                } else if let Some(head) = inp.strip_suffix(*src) {
                    let cand = format!("{head}{dst}");
                    if valid(&cand) && !results.contains(&cand) {
                        results.push(cand);
                    }
                }
            }
        }
        results
    }

    /// 混拼词组查询(正反向),对齐 Python _query_phrase_mixed。
    fn query_phrase_mixed(&self, inp: &str) -> Vec<Candidate> {
        if inp.len() < 2 {
            return Vec::new();
        }
        let segs = self.segment(inp);
        if self.is_full_pinyin(inp) && segs.len() == 1 {
            return Vec::new();
        }
        let mut seen: HashMap<String, i64> = HashMap::new();
        let mut add = |rows: Vec<Candidate>| {
            for (val, w) in rows {
                if val.chars().count() < 2 {
                    continue;
                }
                upsert(&mut seen, val, w);
            }
        };
        if !self.is_full_pinyin(inp) {
            if let Some(mem) = &self.mem {
                add(mem.query_jp(inp, 20));
                let rest = &inp[1..];
                if self.is_full_pinyin(rest) {
                    let first = inp.chars().next().unwrap().to_string();
                    add(mem.query_reverse_mixed(&first, rest, 15));
                }
            } else {
                add(self.db.query_phrase_by_jp(inp, 20).unwrap_or_default());
                let rest = &inp[1..];
                if self.is_full_pinyin(rest) {
                    let first = inp.chars().next().unwrap().to_string();
                    add(self.db.query_phrase_reverse_mixed(&first, rest, 15).unwrap_or_default());
                }
            }
            // 全拼头 + 尾部首字母(2026-11-25 新增):limancx = li man + c x。
            // 这条形态此前按键路径完全没实现 —— 主词库的词打不出来,--add-word 却
            // 提示「混拼都能命中」,属承诺与实现不一致。
            add(self.query_phrase_abbrev_tail(inp));
        }
        // key 前缀:有 mem 走 trie(内存),无则 SQLite 范围查询
        if let Some(mem) = &self.mem {
            let mut kpref: Vec<Candidate> = mem
                .query_prefix(inp)
                .into_iter()
                .filter(|(v, _)| v.chars().count() >= 2)
                .collect();
            kpref.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            kpref.truncate(15);
            add(kpref);
        } else {
            add(self.db.query_phrase_by_key_prefix(inp, 15).unwrap_or_default());
        }
        let mut out: Vec<Candidate> = seen.into_iter().collect();
        // 同 query():加字典序决胜键,消除 HashMap 随机顺序导致的同权重漂移。
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out.truncate(15);
        out
    }

    /// 「全拼头 + 尾部首字母」混拼匹配(2026-11-25)。
    ///
    /// 把输入切成 `(head, tail)`:`head` 当词组 key 的前缀查库,`tail` 必须正好是
    /// key 剩余部分**每个音节的首字母**(最后一个音节允许只打了前缀)。
    ///   例:`limancx` → key=`limancaixiang`;head=`liman`,余下 `caixiang` → 首字母 `cx` ✓
    ///       `yakebicx` → key=`yakebicaixiang`;head=`yakebic`,余下 `aixiang` → `ax` ✗
    ///       `yakebicx` 再拆 head=`yakebi`,余下 `caixiang` → `cx` ✓
    ///
    /// 只走 SQLite(mem 索引的前缀桶只存 value/weight,没有 key 无法校验尾部);
    /// 这是**增量通道**,查不到就返回空,不改变既有 4 条查询路径的任何行为。
    fn query_phrase_abbrev_tail(&self, inp: &str) -> Vec<Candidate> {
        if inp.len() < 4 || !inp.bytes().all(|b| b.is_ascii_lowercase()) {
            return Vec::new();
        }
        let mut out: Vec<Candidate> = Vec::new();
        let mut seen: HashMap<String, i64> = HashMap::new();
        let mut queries = 0;
        // head 从长到短:越长越具体,通常第一刀就命中。
        // 上限 3 刀 + 收满 8 条即停,保证每次按键只多几次索引范围查询。
        for head_len in (3..inp.len()).rev() {
            let (head, tail) = inp.split_at(head_len);
            if tail.is_empty() {
                continue;
            }
            queries += 1;
            if let Ok(rows) = self.db.query_phrase_with_key_prefix(head, 60) {
                for (value, key, weight) in rows {
                    if value.chars().count() < 2 || !key.starts_with(head) {
                        continue;
                    }
                    let remainder = &key[head.len()..];
                    if !remainder.is_empty() && crate::db::abbrev_prefix_match(&self.syll, remainder, tail) {
                        upsert(&mut seen, value, weight);
                    }
                }
            }
            if seen.len() >= 8 || queries >= 3 {
                break;
            }
        }
        out.extend(seen);
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out.truncate(15);
        out
    }

    /// 整句智能首选。
    ///
    /// 2026-11-16 P1:由“音节 DP”改为 **字节跨度网格 + 束搜索**(见 `lattice.rs`)。
    /// 旧实现先用 `segment()` 切音节,简拼/混拼根本切不出音节,导致
    /// `sj`/`wabj`/`zhrmghg` 这类“每字单字母”输入完全无整句结果;新实现直接在
    /// 原样字节串上查库(`phrase.key` / `phrase.jp` / `pinyin.jp` 三条精确路径),
    /// 全拼、简拼、混拼、单字母四种形态共用一套解码。
    pub fn smart_sentence(&self, input: &str) -> Option<String> {
        self.decode(input, 1).into_iter().next().map(|d| d.sentence)
    }

    /// 学习用户选择(更新词频)
    pub fn learn(&self, input: &str, selected: &str) {
        if input.is_empty() || selected.is_empty() {
            return;
        }
        let _ = self.db.add_user_word(input, selected, 1);
    }

    // ── 学习词库管理(Step 2 词库管理窗口,2026-09-04)────────────────────────
    // 直通 db 层 user.db 操作。user.db 打开失败时这些方法静默无效(词库管理窗口
    // 按返回空/false 提示「学习库不可用」),不影响打字。

    /// 列出全部学习词 → [(key, value, weight, seq)] 按学习先后升序。
    pub fn user_list(&self) -> Vec<(String, String, i64, i64)> {
        self.db.list_user_words()
    }

    /// 手动加词(走学成置顶固定权重)。
    pub fn user_add(&self, pinyin: &str, word: &str) -> Result<(), String> {
        self.db.add_user_word(pinyin, word, 1).map_err(|e| e.to_string())
    }

    /// 手工造词:按 `pos` 指定候选位(1 起)。语义见 `db::add_manual_word`。
    pub fn user_add_pos(&self, pinyin: &str, word: &str, pos: i64) -> Result<(), String> {
        self.db.add_manual_word(pinyin, word, pos).map_err(|e| e.to_string())
    }

    /// 中文词 → 自动编码(简拼:每音节首字母)。"北京大学" → "bjdx"。
    ///
    /// 快捷造词窗口和 Agent 批量入库都靠它 —— 用户只给中文,编码不用手打。
    /// 逐字查主库 pinyin 单字表取最高权读音(与词库管理窗口的「拼音」栏同源)。
    pub fn auto_code(&self, word: &str) -> Option<String> {
        let full = self.db.pinyin_for_word(word)?;
        Some(crate::db::CikuDb::to_jianpin(&full))
    }

    /// 中文词 → **全拼**。"北京大学" → "beijingdaxue";算不出来(生僻字)→ None。
    ///
    /// 造词时优先写全拼当主 key(简拼由 `add_manual_word` 自动补别名),
    /// 这样全拼/简拼/混拼三种打法都能命中。
    pub fn word_pinyin(&self, word: &str) -> Option<String> {
        self.db.pinyin_for_word(word)
    }

    /// 删除某条学习词,返回删除行数。
    pub fn user_remove(&self, pinyin: &str, word: &str) -> Result<usize, String> {
        self.db.remove_user_word(pinyin, word).map_err(|e| e.to_string())
    }

    /// 按词本身删除(无需知道编码)。用户清理误学词走这条。
    pub fn user_remove_by_value(&self, word: &str) -> Result<usize, String> {
        self.db.remove_user_word_by_value(word).map_err(|e| e.to_string())
    }

    /// 在用户词层搜索(子串,key/value 任一命中)。空串 = 最近学成的 N 条。
    pub fn user_search(&self, term: &str, limit: usize) -> Vec<(String, String, i64)> {
        self.db.search_user_words(term, limit)
    }

    /// 清空全部学习记录。
    pub fn user_clear(&self) -> Result<(), String> {
        self.db.clear_user_words().map_err(|e| e.to_string())
    }

    /// 搜索主词库(只读)→ [(key, value, weight, source)]。供词库管理窗口「全部词库」查询。
    pub fn search_main_dict(&self, term: &str, limit: usize) -> Vec<(String, String, i64, String)> {
        self.db.search_main_dict(term, limit)
    }

    /// 分页搜索主词库(2026-09-04 灵犀式分页)。
    pub fn search_main_dict_page(&self, term: &str, limit: usize, offset: usize) -> Vec<(String, String, i64, String)> {
        self.db.search_main_dict_page(term, limit, offset)
    }

    /// 主库匹配总条数(分页「共 N 条」)。
    pub fn main_dict_count(&self, term: &str) -> usize {
        self.db.search_main_dict_count(term)
    }

    /// 主库删除词条(2026-09-04 主库可写)。返回删除行数。
    pub fn main_delete_word(&self, pinyin: &str, word: &str) -> Result<usize, String> {
        self.db.main_delete_word(pinyin, word)
    }

    /// 主库 upsert 词条(加词/改权重)。返回 true=成功。
    pub fn main_upsert_word(&self, pinyin: &str, word: &str, weight: i64) -> Result<bool, String> {
        self.db.main_upsert_word(pinyin, word, weight)
    }

    /// 同 key 主库 phrase 最高词频(改权重参考)。
    pub fn main_max_weight(&self, pinyin: &str) -> i64 {
        self.db.dict_max_weight(pinyin)
    }
}

fn upsert(map: &mut HashMap<String, i64>, word: String, w: i64) {
    match map.get(&word) {
        Some(&existing) if existing >= w => {}
        _ => {
            map.insert(word, w);
        }
    }
}

#[cfg(test)]
mod quick_add_tests {
    //! 快捷造词「第 N 位」插位逻辑的纯函数测试(2026-11-24)。
    //! 不碰 DB:证实插位语义本身,DB 侧行为由 db.rs 的用例覆盖。
    use super::*;

    fn cands(v: &[&str]) -> Vec<Candidate> {
        v.iter().map(|s| (s.to_string(), 1)).collect()
    }
    fn words(v: &[Candidate]) -> Vec<String> {
        v.iter().map(|c| c.0.clone()).collect()
    }

    #[test]
    fn empty_manual_keeps_order_and_cap() {
        let out = ImeEngine::splice_manual(cands(&["a", "b", "c"]), &[], 2);
        assert_eq!(words(&out), vec!["a", "b"]);
    }

    #[test]
    fn pos_1_goes_first() {
        let out = ImeEngine::splice_manual(cands(&["a", "b"]), &[("x".into(), 1)], 50);
        assert_eq!(words(&out), vec!["x", "a", "b"]);
    }

    #[test]
    fn pos_3_goes_third() {
        let out = ImeEngine::splice_manual(cands(&["a", "b", "c", "d"]), &[("x".into(), 3)], 50);
        assert_eq!(words(&out), vec!["a", "b", "x", "c", "d"]);
    }

    #[test]
    fn pos_beyond_len_lands_at_tail() {
        let out = ImeEngine::splice_manual(cands(&["a", "b"]), &[("x".into(), 99)], 50);
        assert_eq!(words(&out), vec!["a", "b", "x"]);
    }

    #[test]
    fn same_pos_keeps_input_order() {
        let out = ImeEngine::splice_manual(
            cands(&["a", "b"]),
            &[("x".into(), 2), ("y".into(), 2)],
            50,
        );
        assert_eq!(words(&out), vec!["a", "x", "y", "b"]);
    }

    #[test]
    fn existing_word_is_moved_not_duplicated() {
        // 主库本来就有这条候选 → 摘掉后按 pos 重新插,不能出现两条一样的词。
        let out = ImeEngine::splice_manual(cands(&["a", "x", "b"]), &[("x".into(), 1)], 50);
        assert_eq!(words(&out), vec!["x", "a", "b"]);
        assert_eq!(out.iter().filter(|c| c.0 == "x").count(), 1);
    }

    #[test]
    fn cap_still_enforced_after_insert() {
        let out = ImeEngine::splice_manual(cands(&["a", "b", "c"]), &[("x".into(), 1)], 3);
        assert_eq!(words(&out), vec!["x", "a", "b"]);
        assert!(out.len() <= 3);
    }

    #[test]
    fn manual_only_code_still_yields_candidate() {
        // 主库一个候选都没有(编码是自造词独有)→ 手工词必须能打出来。
        let out = ImeEngine::splice_manual(Vec::new(), &[("独家词".into(), 1)], 50);
        assert_eq!(words(&out), vec!["独家词"]);
    }
}

#[cfg(test)]
mod mixed_tail_tests {
    //! 「全拼头 + 尾部首字母」混拼(2026-11-25 修)。
    //!
    //! 用户实测:造词提示「全拼/简拼/混拼都能命中」,但 `limancx` 在真实按键路径上
    //! 既没有候选、又被中英判定当成英文原样上屏。这里用最小词库复现并锁住:
    //!   - `query()` 必须给出多字词(候选第 1 位就是空格会上的那条);
    //!   - `classify_input()` 必须判拼音(否则 IME 会把候选换成原样字母)。
    use super::*;
    use rusqlite::Connection;

    fn engine_with(rows: &[(&str, &str, &str, i64)]) -> (ImeEngine, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "prisir_mixtail_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("ciku.db");
        {
            let c = Connection::open(&main).unwrap();
            c.execute_batch(
                "CREATE TABLE phrase(jp TEXT, key TEXT, value TEXT, weight INTEGER);
                 CREATE INDEX idx_phrase_key ON phrase(key);
                 CREATE INDEX idx_phrase_jp ON phrase(jp);
                 CREATE TABLE pinyin(jp TEXT, key TEXT, value TEXT, weight INTEGER);",
            )
            .unwrap();
            for (jp, key, value, weight) in rows {
                c.execute(
                    "INSERT INTO phrase(jp,key,value,weight) VALUES(?1,?2,?3,?4)",
                    rusqlite::params![jp, key, value, weight],
                )
                .unwrap();
            }
        }
        let e = ImeEngine::new(main.to_str().unwrap()).expect("open fixture db");
        (e, dir)
    }

    fn fixture() -> (ImeEngine, std::path::PathBuf) {
        engine_with(&[
            ("lmcx", "limancaixiang", "黎曼猜想", 9000),
            ("ykbcx", "yakebicaixiang", "雅可比猜想", 8000),
            ("sx", "shuxue", "数学", 500_000),
        ])
    }

    #[test]
    fn full_head_plus_initials_tail_hits_phrase() {
        let (e, dir) = fixture();
        let got = e.query("limancx");
        assert_eq!(
            got.first().map(|c| c.0.as_str()),
            Some("黎曼猜想"),
            "全拼头(liman)+ 尾部首字母(cx)必须命中,实际 {:?}",
            got.iter().take(5).map(|c| c.0.clone()).collect::<Vec<_>>()
        );
        assert!(e.classify_input("limancx") == crate::lang::LangKind::Pinyin,
            "混拼命中时不能判成英文(判英文会把候选换成原样字母)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mixed_tail_works_for_every_split() {
        let (e, dir) = fixture();
        // 前全拼后简拼 / 全简拼 / 全拼 三种打法都要出同一个词
        for input in ["yakebicx", "ykbcx", "yakebicaixiang"] {
            let got = e.query(input);
            assert_eq!(
                got.first().map(|c| c.0.as_str()),
                Some("雅可比猜想"),
                "{} 应命中雅可比猜想,实际 {:?}",
                input,
                got.iter().take(5).map(|c| c.0.clone()).collect::<Vec<_>>()
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plain_full_pinyin_word_keeps_its_head() {
        let (e, dir) = fixture();
        assert_eq!(e.query("shuxue").first().map(|c| c.0.as_str()), Some("数学"));
        // 前缀输入仍应出整词(key 前缀通道不受新逻辑影响)
        assert_eq!(e.query("shuxu").first().map(|c| c.0.as_str()), Some("数学"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
