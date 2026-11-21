//! 词库访问(rusqlite 读 ciku.db)。逻辑对齐 Python database.py。
//! 表:phrase(jp,key,value,weight) pinyin(jp,key,value,weight) wubi86(key,value,weight)。
//! 注意:pinyin 表上游混入约 31 万词组,查单字必须 LENGTH(value)=1 过滤。

use rusqlite::{Connection, OpenFlags, Result as SqlResult};

/// 主库写互斥(2026-09-04 主库可写):防止用户连点「删/加/改」并发开多个写连接撞 SQLITE_BUSY。
/// 进程内单例;词库管理是低频操作,串行无性能问题。
pub(crate) static MAIN_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub struct CikuDb {
    conn: Connection,
    /// 学习记录独立连接(2026-09-03):写 user.db,不碰主库 ciku.db。
    /// 主库只读 → 文件大小/mtime 永不变 → 索引指纹稳定 → 切换即打不重建。
    /// None = user.db 打开失败(学习静默失效,不影响打字)。
    user_conn: Option<Connection>,
}

/// (value, weight)
pub type Weighted = (String, i64);
/// (key, value, weight)
pub type Row = (String, String, i64);
/// (value, seq) — 学习词:学成置顶固定权重 + 学习次序(位置锁定用)
pub type UserWord = (String, i64);

/// 学成置顶固定权重(2026-09-04):必须大于词库静态权重动态范围(实测 max≈26000),
/// 让用户第一次选某词就一步跳到该拼音下学成词第一梯队。之后 weight 不再变
/// (不再 +1 累积、不做时间衰减)→ 位置永久锁定,解决「偶尔用的词位置老变、
/// 无法固化调用」。多个学成词按 seq(学习先后)稳定排序,先学在前。
pub const LEARN_PIN_WEIGHT: i64 = 100_000;

fn next_prefix(prefix: &str) -> String {
    // shij -> shik,用于 key >= p AND key < next(p) 走索引的范围查询。
    if prefix.is_empty() {
        return prefix.to_string();
    }
    let mut chars: Vec<char> = prefix.chars().collect();
    let mut i = chars.len() as isize - 1;
    while i >= 0 && chars[i as usize] == 'z' {
        i -= 1;
    }
    if i < 0 {
        // 全是 z(或单字母 z):没有可 +1 的前缀字符。拼音 key 全是 ASCII 小写字母,
        // 'z' 之后用 '{'(0x7B,z 的下一个 ASCII)作开区间上界,覆盖所有 z* 音节
        // (z/za/zai/.../zuo/zh/zha/.../zhuo)。2026-09-02 修:旧返回 "za" 把范围缩成
        // [z,za),单字母 z 只剩 1 个候选(在),前缀高权桶失效。
        return "{".to_string();
    }
    chars[i as usize] = std::char::from_u32(chars[i as usize] as u32 + 1).unwrap_or(chars[i as usize]);
    chars[..=i as usize].iter().collect()
}

impl CikuDb {
    pub fn open(path: &str) -> SqlResult<Self> {
        // 主库只读打开(2026-09-03 修「学习写库→索引失效重建 69s」):
        // 之前 Connection::open 是读写模式,即使只 SELECT,SQLite 在某些情况下也会刷
        // 主文件 mtime;更严重的是 learn() 每次上屏 UPDATE/INSERT 直接改主库内容,
        // 大小+mtime 一变,load_or_build_index 的指纹(大小+mtime)就对不上 → 全量重建
        // 322MB 索引(69s)→ 切换输入法后要等几十秒才能打字。
        // 只读打开后主库物理上不可写,指纹永久稳定,索引建好一次永久秒开。
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        // 创建黑白名单表(若不存在)。
        // 2026-11-16 修致命回归:主连接是 READ_ONLY,在它上面执行 DDL 会报
        // "attempt to write a readonly database" → open() 失败 → prisir_ime_load 返回 NULL
        // → 整只输入法哑掉(任何尚未建表的老 ciku.db 都会中招)。
        // 改为独立短时读写连接建表;建表失败(只读介质/无权限)不致命,
        // 仅降级为「黑白名单不可用」,打字/查询不受影响。
        // 兼容老词库:主库里可能已有黑白名单表(2026-11-16 起)。只读介质上建表会失败,
        // 静默忽略——新写入一律走 user.db(见 open_user_db)。
        Self::ensure_blacklist_whitelist_tables(path);
        // 学习记录 + 黑白名单都写 user.db。打开失败不致命(学习/名单静默失效,打字不受影响),
        // 但**路径必须落在用户可写目录**:装到 C:\Program Files 时主库目录只读,
        // 老实现把 user.db 放在主库旁边 → 普通用户永远写不进去 → 频率学习静默失效。
        let user_conn = Self::open_user_db(path).ok();
        Ok(CikuDb { conn, user_conn })
    }

    /// 确保黑白名单表存在。
    ///
    /// **必须在独立的读写连接上做**(主连接是 READ_ONLY,DDL 会直接失败)。
    /// 失败静默:降级为黑白名单不可用,不影响引擎加载与打字。
    fn ensure_blacklist_whitelist_tables(path: &str) {
        let Ok(conn) = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) else {
            return;
        };
        let _ = conn.busy_timeout(std::time::Duration::from_millis(2000));
        let _ = conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS word_blacklist (
                value TEXT PRIMARY KEY,
                reason TEXT,
                added_at TEXT DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS word_whitelist (
                value TEXT PRIMARY KEY,
                reason TEXT,
                added_at TEXT DEFAULT (datetime('now'))
            );",
        );
    }

    /// 打开/创建用户数据库 user.db（学习记录 + 黑白名单），确保表存在。
    ///
    /// **路径选择（2026-11-17 修）**：老实现固定放主库同目录 `<name>.user.db`。
    /// 装到 `C:\Program Files\PrisirIME`（NSIS 默认）时该目录只读（Users=ReadAndExecute），
    /// 于是 `Connection::open` 失败 → `user_conn=None` → **频率学习静默失效**，
    /// 黑白名单也写不进去。现在按顺序选目录：
    ///   1. 环境变量 `PRISIR_IME_DATA`（测试/打包/自定义）
    ///   2. 主库同目录（可写时；开发/stage/绿色版行为不变）
    ///   3. Windows: `%LOCALAPPDATA%\PrisirIME`；Linux: `$XDG_DATA_HOME/prisir`（默认 `~/.local/share/prisir`）
    fn open_user_db(main_path: &str) -> SqlResult<Connection> {
        let dir = Self::user_data_dir(main_path);
        let _ = std::fs::create_dir_all(&dir);
        let stem = std::path::Path::new(main_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("ciku");
        let user_path = dir.join(format!("{stem}.user.db"));
        let uc = Connection::open(&user_path)?;
        uc.execute_batch(
            "CREATE TABLE IF NOT EXISTS user_word(
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                weight INTEGER NOT NULL DEFAULT 0,
                seq INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(key, value)
            );
            CREATE TABLE IF NOT EXISTS word_blacklist (
                value TEXT PRIMARY KEY,
                reason TEXT,
                added_at TEXT DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS word_whitelist (
                value TEXT PRIMARY KEY,
                reason TEXT,
                added_at TEXT DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);",
        )?;
        // 对 2026-09-04 之前建的旧 user.db(无 seq 列)补列;已存在则忽略错误。
        let _ = uc.execute_batch("ALTER TABLE user_word ADD COLUMN seq INTEGER NOT NULL DEFAULT 0;");
        eprintln!("[prisir_ime] user db: {}", user_path.display());
        // 老安装(2026-11-17 之前)把 user.db 放在主库旁边。位置一变就得搬一次，
        // 否则用户之前学过的词(可能几千条)全部"消失"——用户感知就是"越用越不准了"。
        Self::migrate_legacy_user_db(main_path, &uc, &user_path);
        Ok(uc)
    }

    /// 把老位置的 user.db（主库同目录）并入新位置。只做一次：搬完把老文件改名 `.migrated`。
    fn migrate_legacy_user_db(main_path: &str, uc: &Connection, chosen: &std::path::Path) {
        let main_dir = std::path::Path::new(main_path).parent().unwrap_or_else(|| std::path::Path::new("."));
        let stem = std::path::Path::new(main_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("ciku");
        let legacy = main_dir.join(format!("{stem}.user.db"));
        if legacy == chosen || !legacy.exists() {
            return;
        }
        // 每次加载都合并一次（INSERT OR IGNORE，幂等，几千行 ~10ms）。
        // 为什么不用"搬过就不再搬"的标记：老版本 DLL 还在跑时仍会往老位置写
        // （用户升级前的这段时间学的词），标记会把这些词永久漏掉。
        // 幂等合并的代价远小于"用户学过的词莫名消失"。
        let Ok(old) = Connection::open_with_flags(&legacy, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
            return;
        };
        let mut moved = 0usize;
        for (table, cols) in [
            ("user_word", "key,value,weight,seq"),
            ("word_blacklist", "value,reason"),
            ("word_whitelist", "value,reason"),
        ] {
            let sql = format!("INSERT OR IGNORE INTO main.{table} ({cols}) SELECT {cols} FROM old.{table}");
            let _ = uc.execute_batch(&format!("ATTACH DATABASE '{}' AS old;", legacy.display()));
            if uc.execute(&sql, []).is_ok() {
                moved += 1;
            }
            let _ = uc.execute_batch("DETACH DATABASE old;");
        }
        if moved > 0 {
            // 主库目录只读时改名会失败（改名要目录写权限）——失败没关系，下次会再合并一遍
            let _ = std::fs::rename(&legacy, legacy.with_extension("migrated"));
            eprintln!("[prisir_ime] 已把老 user.db 并入用户数据目录: {}", legacy.display());
        }
    }

    /// 用户可写目录（见 open_user_db 注释）。
    fn user_data_dir(main_path: &str) -> std::path::PathBuf {
        use std::path::{Path, PathBuf};
        if let Ok(p) = std::env::var("PRISIR_IME_DATA") {
            if !p.trim().is_empty() {
                return PathBuf::from(p);
            }
        }
        let main_dir = Path::new(main_path).parent().unwrap_or_else(|| Path::new("."));
        if Self::dir_writable(main_dir) {
            return main_dir.to_path_buf();
        }
        #[cfg(windows)]
        {
            if let Ok(local) = std::env::var("LOCALAPPDATA") {
                if !local.trim().is_empty() {
                    return Path::new(&local).join("PrisirIME");
                }
            }
            if let Ok(profile) = std::env::var("USERPROFILE") {
                return Path::new(&profile).join("AppData").join("Local").join("PrisirIME");
            }
        }
        #[cfg(not(windows))]
        {
            if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
                if !xdg.trim().is_empty() {
                    return Path::new(&xdg).join("prisir");
                }
            }
            if let Ok(home) = std::env::var("HOME") {
                return Path::new(&home).join(".local").join("share").join("prisir");
            }
        }
        main_dir.to_path_buf()
    }

    /// 目录可写探测（建一个探针文件再删）。
    fn dir_writable(dir: &std::path::Path) -> bool {
        let probe = dir.join(".prisir_write_probe");
        match std::fs::File::create(&probe) {
            Ok(_) => {
                let _ = std::fs::remove_file(&probe);
                true
            }
            Err(_) => false,
        }
    }

    /// 下一个学习次序号(现有 max(seq)+1,从 1 起)。学成词按 seq 升序锁定位置。
    fn next_seq(uc: &Connection) -> i64 {
        uc.query_row("SELECT COALESCE(MAX(seq),0)+1 FROM user_word", [], |r| r.get(0))
            .unwrap_or(1)
    }

    /// 全部词组 (key,value,weight),供内存索引构建。
    pub fn all_phrase_rows(&self) -> SqlResult<Vec<Row>> {
        let mut st = self.conn.prepare("SELECT key, value, weight FROM phrase")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.filter_map(|x| x.ok()).collect())
    }

    /// 全部词组带简拼 (jp,key,value,weight),供混拼内存索引构建(2026-09-02)。
    pub fn all_phrase_rows_with_jp(&self) -> SqlResult<Vec<(String, String, String, i64)>> {
        let mut st = self.conn.prepare("SELECT jp, key, value, weight FROM phrase")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        Ok(rows.filter_map(|x| x.ok()).collect())
    }

    /// 全部单字 (key,value,weight),LENGTH(value)=1。
    pub fn all_single_char_rows(&self) -> SqlResult<Vec<Row>> {
        let mut st = self
            .conn
            .prepare("SELECT key, value, weight FROM pinyin WHERE LENGTH(value)=1")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.filter_map(|x| x.ok()).collect())
    }

    /// 拼音单字(过滤多字词组)。
    pub fn query_pinyin(&self, key: &str, limit: usize) -> SqlResult<Vec<Weighted>> {
        let mut st = self.conn.prepare(
            "SELECT value, weight FROM pinyin WHERE key=?1 AND LENGTH(value)=1 ORDER BY weight DESC LIMIT ?2",
        )?;
        let rows = st.query_map(rusqlite::params![key, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.filter_map(|x| x.ok()).collect())
    }

    /// 简拼单字(过滤多字词组)。
    pub fn query_pinyin_jp(&self, jp: &str, limit: usize) -> SqlResult<Vec<Weighted>> {
        let mut st = self.conn.prepare(
            "SELECT value, weight FROM pinyin WHERE jp=?1 AND LENGTH(value)=1 ORDER BY weight DESC LIMIT ?2",
        )?;
        let rows = st.query_map(rusqlite::params![jp, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.filter_map(|x| x.ok()).collect())
    }

    /// 单字母前缀高权单字桶(2026-09-02):key 以该字母开头的所有音节里,按权重取高权单字。
    /// 对齐外挂内存路径的「前缀节点高权桶」语义(z→在/张/中/这/再…),取代只查 jp='z' 的窄覆盖。
    /// 范围比较 key>=p AND key<next(p) 走索引;过滤 LENGTH(value)=1 防词组混入。
    pub fn query_pinyin_prefix_top(&self, prefix: &str, limit: usize) -> SqlResult<Vec<Weighted>> {
        let mut st = self.conn.prepare(
            "SELECT value, weight FROM pinyin WHERE key >= ?1 AND key < ?2 AND LENGTH(value)=1 \
             ORDER BY weight DESC LIMIT ?3",
        )?;
        let rows = st.query_map(
            rusqlite::params![prefix, next_prefix(prefix), limit as i64],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(rows.filter_map(|x| x.ok()).collect())
    }

    /// 词组全拼 key 精确。
    pub fn query_phrase(&self, key: &str, limit: usize) -> SqlResult<Vec<Weighted>> {
        let mut st = self
            .conn
            .prepare("SELECT value, weight FROM phrase WHERE key=?1 ORDER BY weight DESC LIMIT ?2")?;
        let rows = st.query_map(rusqlite::params![key, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.filter_map(|x| x.ok()).collect())
    }

    /// 纯简拼词组:jp 精确(sj→世界/时间)。
    pub fn query_phrase_by_jp(&self, jp: &str, limit: usize) -> SqlResult<Vec<Weighted>> {
        let mut st = self
            .conn
            .prepare("SELECT value, weight FROM phrase WHERE jp=?1 ORDER BY weight DESC LIMIT ?2")?;
        let rows = st.query_map(rusqlite::params![jp, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.filter_map(|x| x.ok()).collect())
    }

    /// 纯简拼词组:jp 精确,**并带出词库里的全拼 key**(jp→key→value)。
    ///
    /// 用途(2026-11-19):26 键打简拼("nhjsg" → 「你好旧时光」)本来就能出多字词,
    /// 但界面第一行只显示用户敲的字母,用户不知道它被解读成了什么 → 功能等于不存在。
    /// 候选查询只需要 (词, 权重),**读音**只有这里才有 → 单独开一个接口,
    /// 避免给常规候选查询带上多余字段(每次按键都要查)。
    ///
    /// 走 idx_phrase_jp 索引(打包时随 jp 列一起建),实测 ~1ms。
    pub fn query_phrase_jp_reading(
        &self,
        jp: &str,
        limit: usize,
    ) -> SqlResult<Vec<(String, String, i64)>> {
        let mut st = self
            .conn
            .prepare("SELECT key, value, weight FROM phrase WHERE jp=?1 ORDER BY weight DESC LIMIT ?2")?;
        let rows = st.query_map(rusqlite::params![jp, limit as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        Ok(rows.filter_map(|x| x.ok()).collect())
    }

    /// 词 → 词库里的全拼 key（走 idx_phrase_value 索引，~1ms）。
    ///
    /// 用途(2026-11-19)：候选栏第一行要显示**这条候选的读音**（"你好" → "ni hao"）。
    /// 必须按**词**反查，不能按输入反查 —— 按输入反查会和候选排序打架：
    /// 输入 "nhxf" 时 jp 精确命中的最高权重行是「南湖蟹粉」，但候选排序把
    /// 「你好幸福」排在前面 → 显示的读音和首候选对不上，用户会懵。
    /// 同一个词有多条 phrase 行（多音字/不同读法）时取权重最高的那条。
    pub fn query_phrase_key_for_word(&self, word: &str) -> Option<String> {
        if word.is_empty() {
            return None;
        }
        self.conn
            .query_row(
                "SELECT key FROM phrase WHERE value=?1 ORDER BY weight DESC LIMIT 1",
                rusqlite::params![word],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .filter(|k: &String| !k.is_empty())
    }

    /// 前全拼+后首字母:key 前缀(shij→世界/实际)。范围比较走 idx_phrase_key。
    pub fn query_phrase_by_key_prefix(&self, prefix: &str, limit: usize) -> SqlResult<Vec<Weighted>> {
        let mut st = self.conn.prepare(
            "SELECT value, weight FROM phrase WHERE key >= ?1 AND key < ?2 ORDER BY weight DESC LIMIT ?3",
        )?;
        let rows = st.query_map(
            rusqlite::params![prefix, next_prefix(prefix), limit as i64],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(rows.filter_map(|x| x.ok()).collect())
    }

    /// 反向混拼:jp 首字母=first(范围),结果集内过滤 key 后缀 rest(sjie→世界)。
    pub fn query_phrase_reverse_mixed(&self, first: &str, rest: &str, limit: usize) -> SqlResult<Vec<Weighted>> {
        // 首字母 + 后全拼（sjie → 世界）。这条是**唯一**的百毫秒级慢查询：
        // 原 SQL 只按 jp 首字母范围扫（z 开头的 17 万行）再按权重排序 → 'zo' 341ms / 'zhongguo' 658ms。
        //
        // 两刀收紧（2026-11-17，实测）：
        //  1) length(jp) = 音节数 = len(rest)+1 —— jp 每位对应一个音节，长度是必要条件。
        //     'zo' 341ms → 30ms（z 开头 17.7 万行 → 长度 2 的 1.2 万行）
        //  2) rest 长度 ≥3 时用 substr(key,-3) 作为前置过滤（打包期建 idx_phrase_key_tail3）。
        //     'sjie' 131ms → 45ms。key 以 rest 结尾 ⇒ key 末 3 字必等于 rest 末 3 字（必要条件，
        //     不会漏；最后仍有 Rust 侧 ends_with 兜底保证正确）。
        let n_syll = (rest.chars().count() + 1) as i64;
        let use_tail3 = rest.chars().count() >= 3;
        let tail3: String = if use_tail3 {
            let v: Vec<char> = rest.chars().rev().take(3).collect();
            v.into_iter().rev().collect()
        } else {
            String::new()
        };
        let sql = if use_tail3 {
            "SELECT value, key, weight FROM phrase              WHERE substr(key,-3) = ?1 AND jp >= ?2 AND jp < ?3 AND length(jp) = ?4              ORDER BY weight DESC LIMIT ?5"
        } else {
            "SELECT value, key, weight FROM phrase              WHERE jp >= ?1 AND jp < ?2 AND length(jp) = ?3              ORDER BY weight DESC LIMIT ?4"
        };
        let mut st = self.conn.prepare(sql)?;
        let rows: Vec<(String, String, i64)> = if use_tail3 {
            st.query_map(
                rusqlite::params![tail3, first, next_prefix(first), n_syll, (limit * 20) as i64],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?
            .filter_map(|x| x.ok())
            .collect()
        } else {
            st.query_map(
                rusqlite::params![first, next_prefix(first), n_syll, (limit * 20) as i64],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?
            .filter_map(|x| x.ok())
            .collect()
        };
        let mut out: Vec<Weighted> = rows
            .into_iter()
            .filter(|(_, key, _)| key.ends_with(rest))
            .map(|(val, _, w)| (val, w))
            .collect();
        out.truncate(limit);
        Ok(out)
    }

    /// 九宫格（T9）候选：按数字串查 `t9` 表（精确 + 前缀），权重降序。
    ///
    /// 表由 `lexicon_pipeline/build_t9.py` 生成（460k 行 / +25MB）：
    /// 2=abc 3=def 4=ghi 5=jkl 6=mno 7=pqrs 8=tuv 9=wxyz，词条拼音整体转数字串。
    /// 运行期展开数字→字母组合不可行（4^L），所以必须建表。
    pub fn query_t9(&self, digits: &str, limit: usize) -> SqlResult<Vec<Weighted>> {
        let mut out: Vec<Weighted> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        // 1) 精确整串（打完这个词的全部数字）
        if let Ok(mut st) = self
            .conn
            .prepare("SELECT value, weight FROM t9 WHERE t9=?1 ORDER BY weight DESC LIMIT ?2")
        {
            if let Ok(rows) = st.query_map(rusqlite::params![digits, limit as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            }) {
                for (v, w) in rows.flatten() {
                    if seen.insert(v.clone()) {
                        out.push((v, w));
                    }
                }
            }
        }
        // 2) 前缀（还没打完，或者打的是更长词的开头）
        if out.len() < limit {
            if let Ok(mut st) = self.conn.prepare(
                "SELECT value, weight FROM t9 WHERE t9 >= ?1 AND t9 < ?2 ORDER BY weight DESC LIMIT ?3",
            ) {
                if let Ok(rows) = st.query_map(
                    rusqlite::params![digits, next_prefix(digits), limit as i64],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
                ) {
                    for (v, w) in rows.flatten() {
                        if out.len() >= limit {
                            break;
                        }
                        if seen.insert(v.clone()) {
                            out.push((v, w));
                        }
                    }
                }
            }
        }
        out.truncate(limit);
        Ok(out)
    }

    /// 由中文词反推全拼（逐字查 pinyin 单字表，取每字最高权读音）。
    ///
    /// 用途：用户「加入词库」时只需要打中文，拼音由引擎算 —— Windows 词库窗、
    /// Android 词库管理、Linux CLI 都不用自己实现汉字→拼音。
    pub fn pinyin_for_word(&self, word: &str) -> Option<String> {
        if word.is_empty() {
            return None;
        }
        let mut key = String::new();
        for ch in word.chars() {
            let v = ch.to_string();
            let k: Option<String> = self
                .conn
                .query_row(
                    "SELECT key FROM pinyin WHERE value=?1 ORDER BY weight DESC LIMIT 1",
                    rusqlite::params![v],
                    |r| r.get(0),
                )
                .ok();
            match k {
                Some(k) if !k.is_empty() => key.push_str(&k),
                _ => return None,
            }
        }
        if key.is_empty() { None } else { Some(key) }
    }

    /// 五笔候选(预留)。
    pub fn query_wubi(&self, key: &str, limit: usize) -> SqlResult<Vec<Weighted>> {
        let mut st = self
            .conn
            .prepare("SELECT value, weight FROM wubi86 WHERE key=?1 ORDER BY weight DESC LIMIT ?2")?;
        let rows = st.query_map(rusqlite::params![key, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.filter_map(|x| x.ok()).collect())
    }

    /// 学习:写独立 user.db 的 user_word 表(2026-09-03 改,不再碰主库 phrase/pinyin)。
    /// 主库只读后,任何 UPDATE/INSERT 主库都会返错;学习记录统一进 user.db,
    /// 查询时由 engine 合并(学成词置顶)。user_conn 为 None(打开失败)时静默跳过。
    ///
    /// 2026-09-04 学成置顶+位置锁定:
    /// - 新词首次学:weight 固定 = LEARN_PIN_WEIGHT(不再从 1 起累积),seq 取学习次序。
    /// - 已学词再选:**只刷新 seq 不动 weight** — 权重不累积 → 学成词位置永久锁定,
    ///   解决「偶尔用的词位置老变、无法固化调用」(用户 2026-09-04 明确不要时间衰减)。
    /// `weight` 参数保留兼容旧调用(传 1),本函数内部忽略之,统一用 LEARN_PIN_WEIGHT。
    pub fn add_user_word(&self, pinyin: &str, word: &str, _weight: i64) -> SqlResult<()> {
        let Some(uc) = &self.user_conn else {
            return Ok(()); // user.db 不可用:学习失效但不影响打字
        };
        let seq = Self::next_seq(uc);
        uc.execute(
            "INSERT INTO user_word(key,value,weight,seq) VALUES(?1,?2,?3,?4)
             ON CONFLICT(key,value) DO UPDATE SET seq=excluded.seq",
            rusqlite::params![pinyin, word, LEARN_PIN_WEIGHT, seq],
        )?;
        Ok(())
    }

    /// 查学习词:某拼音**前缀**下用户学过的所有词 → [(value, seq)](按 seq 升序,先学在前)。
    ///
    /// 用途(2026-11-20)：九宫格按**读音**查学成词，而学习时记的 key 是**词的完整读音**
    /// （"米高扬" → "migaoyang"）。九宫格读音 "mi gao" 只是它的一个**前缀** →
    /// 精确匹配查不到 → 学成词在九宫格里不置顶（用户感知："刚选过的词又不见了"）。
    /// 用范围比较(key >= p AND key < next(p))而不是 LIKE → 能走索引,也避开 %/_ 通配符转义。
    pub fn user_words_with_prefix(&self, prefix: &str) -> Vec<UserWord> {
        let Some(uc) = &self.user_conn else {
            return Vec::new();
        };
        let mut st = match uc
            .prepare("SELECT value, seq FROM user_word WHERE key>=?1 AND key<?2 ORDER BY seq ASC")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = match st.query_map(rusqlite::params![prefix, next_prefix(prefix)], |r| {
            Ok((r.get(0)?, r.get(1)?))
        }) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(|x| x.ok()).collect()
    }

    /// 查学习词:某拼音下用户学过的所有词 → [(value, seq)](按 seq 升序,先学在前)。
    /// 供 engine 查询合并用(学成置顶+位置锁定)。无 user.db 时返空。
    /// 返回的 weight 由 engine 统一给 LEARN_PIN_WEIGHT,这里只带 seq 用于稳定排序。
    pub fn user_words_for(&self, pinyin: &str) -> Vec<UserWord> {
        let Some(uc) = &self.user_conn else {
            return Vec::new();
        };
        let mut st = match uc.prepare("SELECT value, seq FROM user_word WHERE key=?1 ORDER BY seq ASC") {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = match st.query_map(rusqlite::params![pinyin], |r| Ok((r.get(0)?, r.get(1)?))) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(|x| x.ok()).collect()
    }

    /// 删除指定学习词(词库管理窗口用)。返受影响行数。
    pub fn remove_user_word(&self, pinyin: &str, word: &str) -> SqlResult<usize> {
        let Some(uc) = &self.user_conn else {
            return Ok(0);
        };
        uc.execute(
            "DELETE FROM user_word WHERE key=?1 AND value=?2",
            rusqlite::params![pinyin, word],
        )
    }

    /// 清空全部学习记录(词库管理窗口「清空学习」用)。
    pub fn clear_user_words(&self) -> SqlResult<()> {
        let Some(uc) = &self.user_conn else {
            return Ok(());
        };
        uc.execute("DELETE FROM user_word", [])?;
        Ok(())
    }

    /// 搜索主词库(只读,词库管理窗口「全部词库」查询用,2026-09-04)。
    /// term 纯 ASCII → 拼音 key 前缀;含中文 → 词 value 前缀(2026-11-17 从子串改前缀,见下方注释)。
    /// 返回 [(key, value, weight, source)],source ∈ phrase/pinyin。主库只读,只能查不能改。
    pub fn search_main_dict(&self, term: &str, limit: usize) -> Vec<(String, String, i64, String)> {
        self.search_main_dict_page(term, limit, 0)
    }

    /// 分页搜索(词库管理窗口用)。
    ///
    /// **2026-11-17 改为前缀匹配**（原来 `key LIKE '%k%' OR value LIKE '%k%'`）：
    /// 子串匹配在有 190 万行的 phrase 上必须全表扫 + 排序，实测**单次 3.1 秒**、
    /// 加上"共 N 条"的 count 要两次全扫 = 词库窗一点搜索就冻 5 秒。
    /// 前缀匹配走索引（`idx_phrase_key` / `idx_phrase_value`）后 **1ms**。
    ///
    /// 语义：
    ///   - keyword 纯 ASCII → 拼音前缀（banquan → 版权/版权局/版权法…）
    ///   - keyword 含中文   → 词语前缀（版权 → 版权/版权局/版权法…）
    ///   - keyword 空       → 按权重全库 top（初始铺一批高频词）
    /// 不支持"词中间"匹配（版权局 搜 权局）：那必须全表扫，代价 > 收益。
    pub fn search_main_dict_page(&self, term: &str, limit: usize, offset: usize) -> Vec<(String, String, i64, String)> {
        let t = term.trim();
        let mut out: Vec<(String, String, i64, String)> = Vec::new();
        if t.is_empty() {
            if let Ok(mut st) = self.conn.prepare(
                "SELECT key, value, weight FROM phrase ORDER BY weight DESC LIMIT ?1 OFFSET ?2",
            ) {
                if let Ok(rows) = st.query_map(rusqlite::params![limit as i64, offset as i64], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
                }) {
                    for r in rows.flatten() {
                        out.push((r.0, r.1, r.2, "phrase".to_string()));
                    }
                }
            }
            out.truncate(limit);
            return out;
        }
        let col = if t.is_ascii() { "key" } else { "value" };
        let sql = format!(
            "SELECT key, value, weight FROM phrase WHERE {col} >= ?1 AND {col} < ?2              ORDER BY weight DESC LIMIT ?3 OFFSET ?4"
        );
        if let Ok(mut st) = self.conn.prepare(&sql) {
            if let Ok(rows) = st.query_map(
                rusqlite::params![t, next_prefix(t), limit as i64, offset as i64],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)),
            ) {
                for r in rows.flatten() {
                    out.push((r.0, r.1, r.2, "phrase".to_string()));
                }
            }
        }
        out.truncate(limit);
        out
    }

    /// 匹配总条数(分页「共 N 条」用)。空 = 全表总数。与 search_main_dict_page 同语义(前缀)。
    pub fn search_main_dict_count(&self, term: &str) -> usize {
        let t = term.trim();
        let total: i64 = if t.is_empty() {
            self.conn.query_row("SELECT COUNT(*) FROM phrase", [], |r| r.get::<_, i64>(0)).unwrap_or(0)
        } else {
            let col = if t.is_ascii() { "key" } else { "value" };
            let sql = format!("SELECT COUNT(*) FROM phrase WHERE {col} >= ?1 AND {col} < ?2");
            self.conn
                .query_row(&sql, rusqlite::params![t, next_prefix(t)], |r| r.get::<_, i64>(0))
                .unwrap_or(0)
        };
        total.max(0) as usize
    }

    /// 列出全部学习词(词库管理窗口列表用) → [(key, value, weight, seq)] 按 seq 升序。
    pub fn list_user_words(&self) -> Vec<(String, String, i64, i64)> {
        let Some(uc) = &self.user_conn else {
            return Vec::new();
        };
        let mut st = match uc.prepare("SELECT key, value, weight, seq FROM user_word ORDER BY seq ASC") {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = match st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(|x| x.ok()).collect()
    }

    // ============================================================
    // 主库可写(2026-09-04,恢复外挂式删/加/改权重,用户拍板)
    //
    // 语义复用 lingxi_ime/backend/database.py:
    //   dict_delete(pinyin,word)        → DELETE FROM phrase WHERE key AND value
    //   dict_upsert(pinyin,word,weight) → 存在改 weight+jp,不存在 INSERT(jp 由音节首字母算)
    //   dict_max_weight(pinyin)         → 同 key 最高词频(新词提频排第一)
    //
    // **架构(多进程安全)**:打字引擎保持只读(切换秒开),主库写**不走 self.conn**,
    // 每次写都独立开一个短时读写连接(开→写→立即 Drop 关),配 busy_timeout 等读锁,
    // 并由 MAIN_WRITE_LOCK 进程内互斥。这样多宿主进程(notepad/explorer)各自只读
    // 不受影响,只有词库管理这一个低频操作短时持写锁。
    //
    // **一致性**:运行时引擎纯 SQLite(mem 索引恒 None,见 engine.rs load_or_build_index),
    // 所以写后主库内容即刻对新查询生效;但同进程 self.conn 是只读旧连接,新建查询
    // 连接才读到最新。失效的 .idx/.midx 缓存索引由 invalidate_main_index_caches 清理。
    // ============================================================

    /// 主库路径(从只读连接的 db 文件名取,供短时写连接用)。
    fn main_db_path(&self) -> SqlResult<String> {
        self.conn
            .query_row("PRAGMA database_list", [], |r| r.get::<_, String>(2))
            .map_err(|e| e)
    }

    /// 开一个短时主库读写连接(busy_timeout 5s 等读锁释放)。调用方须立即用立即 Drop。
    fn open_main_writable(&self) -> SqlResult<Connection> {
        let path = self.main_db_path()?;
        let c = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        c.busy_timeout(std::time::Duration::from_millis(5000));
        Ok(c)
    }

    /// 同 key 下 phrase 最高词频(改权重/新词提频参考)。词库管理 UI 用。
    pub fn dict_max_weight(&self, pinyin: &str) -> i64 {
        self.conn
            .query_row(
                "SELECT MAX(weight) FROM phrase WHERE key=?1",
                rusqlite::params![pinyin],
                |r| r.get(0),
            )
            .ok()
            .flatten()
            .unwrap_or(0)
    }

    /// 删除主库词条(phrase)。返回删除行数。多进程安全(独立短时可写连接+互斥)。
    pub fn main_delete_word(&self, pinyin: &str, word: &str) -> Result<usize, String> {
        // 只读安装（C:\Program Files / /usr/lib）写不了主库 → 退化为"用户层删除":
        // 记进黑名单（候选里不再出现）+ 清掉同名学习词。可逆（点「解除」就恢复）。
        let Ok(c) = self.open_main_writable() else {
            let n = self.add_to_blacklist(word, "用户删除")?;
            let _ = self.remove_user_word(pinyin, word);
            self.invalidate_main_index_caches();
            return Ok(n.max(1));
        };
        let _g = MAIN_WRITE_LOCK.lock().map_err(|e| e.to_string())?;
        let n = c
            .execute(
                "DELETE FROM phrase WHERE key=?1 AND value=?2",
                rusqlite::params![pinyin, word],
            )
            .map_err(|e| e.to_string())?;
        drop(c); // 立即关写连接,释放写锁
        self.invalidate_main_index_caches();
        Ok(n)
    }

    /// 主库 upsert 词条(phrase):存在改 weight+jp,不存在插入。
    /// 外挂「改权重」= 对同 (key,value) upsert 新 weight(等效先删旧词再插新权重词)。
    /// 返回 true=成功。
    pub fn main_upsert_word(&self, pinyin: &str, word: &str, weight: i64) -> Result<bool, String> {
        // 只读安装：写用户层 —— 加进 user_word（学成置顶，等价于"我加的词必须排前面"），
        // 并解除它可能存在的"用户删除"黑名单记录。UI 上就是"保存成功"。
        if self.open_main_writable().is_err() {
            self.add_user_word(pinyin, word, weight).map_err(|e| e.to_string())?;
            let _ = self.remove_from_blacklist(word);
            self.invalidate_main_index_caches();
            return Ok(true);
        }
        let _g = MAIN_WRITE_LOCK.lock().map_err(|e| e.to_string())?;
        let c = self.open_main_writable().map_err(|e| e.to_string())?;
        let jp = Self::to_jianpin(pinyin);
        let exists: bool = c
            .query_row(
                "SELECT 1 FROM phrase WHERE key=?1 AND value=?2",
                rusqlite::params![pinyin, word],
                |_| Ok(()),
            )
            .is_ok();
        if exists {
            c.execute(
                "UPDATE phrase SET weight=?1, jp=?2 WHERE key=?3 AND value=?4",
                rusqlite::params![weight, jp, pinyin, word],
            )
            .map_err(|e| e.to_string())?;
        } else {
            c.execute(
                "INSERT INTO phrase (jp, key, value, weight) VALUES (?1,?2,?3,?4)",
                rusqlite::params![jp, pinyin, word, weight],
            )
            .map_err(|e| e.to_string())?;
        }
        drop(c);
        self.invalidate_main_index_caches();
        Ok(true)
    }

    /// 写主库后清缓存索引(.idx/.midx):指纹已变,留着只会让别的进程误判或重建。
    /// 运行时纯 SQLite,删掉它们无副作用(下次需要时按需重建,但当前链不用)。
    fn invalidate_main_index_caches(&self) {
        if let Ok(path) = self.main_db_path() {
            for ext in ["idx", "midx"] {
                let p = std::path::Path::new(&path).with_extension(ext);
                let _ = std::fs::remove_file(p);
            }
        }
    }

    /// 拼音转简拼(每音节首字母),对齐 database.py _to_jianpin。
    /// 用标准音节表切分后取各音节首字母;切不出则退化为整串首字母。
    pub fn to_jianpin(pinyin: &str) -> String {
        let sy = crate::syllable::Syllables::new();
        let segs = sy.segment(pinyin);
        if segs.is_empty() {
            return pinyin.chars().next().map(|c| c.to_string()).unwrap_or_default();
        }
        segs.iter().filter_map(|s| s.chars().next()).collect()
    }

    // ============================================================
    // 黑白名单 CRUD (主库可写,独立短时连接)
    // ============================================================

    /// 判断词组是否在黑名单中。
    /// 判断词组是否在黑名单中。
    ///
    /// **读：user.db 优先，再查主库**（兼容 2026-11-16 那版把名单写在主库里的老词库）。
    /// **写：一律 user.db** —— 装到 Program Files 时主库目录只读，写主库必然失败。
    /// 一次性读出**黑名单全集**(user.db ∪ 主库)—— 2026-11-19 新增。
    ///
    /// 背景:候选过滤原来对**每个候选词**调 `is_blacklisted`/`is_whitelisted`,
    /// 而这两个点查各查两个库(user.db ∪ 主库)→ 50 个候选 = 200 次 SQL,
    /// 单次 query 的 8~11ms 大头就在这里(与候选桶本身快慢无关)。
    /// 名单表本身极小(几十条),一次性读进 HashSet 再查,快两个数量级。
    pub fn blacklist_set(&self) -> std::collections::HashSet<String> {
        let mut set = std::collections::HashSet::new();
        if let Some(uc) = &self.user_conn {
            if let Ok(mut st) = uc.prepare("SELECT value FROM word_blacklist") {
                if let Ok(rows) = st.query_map([], |r| r.get::<_, String>(0)) {
                    for v in rows.flatten() {
                        set.insert(v);
                    }
                }
            }
        }
        if let Ok(mut st) = self.conn.prepare("SELECT value FROM word_blacklist") {
            if let Ok(rows) = st.query_map([], |r| r.get::<_, String>(0)) {
                for v in rows.flatten() {
                    set.insert(v);
                }
            }
        }
        set
    }

    /// 一次性读出**白名单全集**(user.db ∪ 主库)—— 同 blacklist_set。
    pub fn whitelist_set(&self) -> std::collections::HashSet<String> {
        let mut set = std::collections::HashSet::new();
        if let Some(uc) = &self.user_conn {
            if let Ok(mut st) = uc.prepare("SELECT value FROM word_whitelist") {
                if let Ok(rows) = st.query_map([], |r| r.get::<_, String>(0)) {
                    for v in rows.flatten() {
                        set.insert(v);
                    }
                }
            }
        }
        if let Ok(mut st) = self.conn.prepare("SELECT value FROM word_whitelist") {
            if let Ok(rows) = st.query_map([], |r| r.get::<_, String>(0)) {
                for v in rows.flatten() {
                    set.insert(v);
                }
            }
        }
        set
    }

    pub fn is_blacklisted(&self, value: &str) -> bool {
        if let Some(uc) = &self.user_conn {
            if uc
                .query_row("SELECT 1 FROM word_blacklist WHERE value=?1", rusqlite::params![value], |_| Ok(true))
                .unwrap_or(false)
            {
                return true;
            }
        }
        self.conn
            .query_row("SELECT 1 FROM word_blacklist WHERE value=?1", rusqlite::params![value], |_| Ok(true))
            .unwrap_or(false)
    }

    /// 判断词组是否在白名单中。
    pub fn is_whitelisted(&self, value: &str) -> bool {
        if let Some(uc) = &self.user_conn {
            if uc
                .query_row("SELECT 1 FROM word_whitelist WHERE value=?1", rusqlite::params![value], |_| Ok(true))
                .unwrap_or(false)
            {
                return true;
            }
        }
        self.conn
            .query_row("SELECT 1 FROM word_whitelist WHERE value=?1", rusqlite::params![value], |_| Ok(true))
            .unwrap_or(false)
    }

    /// 读一张名单（user.db ∪ 主库，按 value 去重）。
    fn read_name_list(&self, table: &str) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let sql = format!("SELECT value, reason FROM {table}");
        for conn in [self.user_conn.as_ref(), Some(&self.conn)].into_iter().flatten() {
            if let Ok(mut st) = conn.prepare(&sql) {
                if let Ok(rows) = st.query_map([], |r| {
                    Ok((r.get::<_, String>(0).unwrap_or_default(), r.get::<_, String>(1).unwrap_or_default()))
                }) {
                    for (v, reason) in rows.flatten() {
                        if seen.insert(v.clone()) {
                            out.push((v, reason));
                        }
                    }
                }
            }
        }
        out
    }

    /// 获取全部黑名单 [(value, reason)]。
    pub fn get_blacklist(&self) -> Vec<(String, String)> {
        self.read_name_list("word_blacklist")
    }

    /// 获取全部白名单 [(value, reason)]。
    pub fn get_whitelist(&self) -> Vec<(String, String)> {
        self.read_name_list("word_whitelist")
    }

    /// 名单写入统一入口：只写 user.db（主库只读）。
    fn write_name_list(&self, table: &str, value: &str, reason: Option<&str>) -> Result<usize, String> {
        let uc = self
            .user_conn
            .as_ref()
            .ok_or_else(|| "用户数据目录不可写（user.db 打不开）".to_string())?;
        let _g = MAIN_WRITE_LOCK.lock().map_err(|e| e.to_string())?;
        let n = match reason {
            Some(r) => uc.execute(
                // OR REPLACE 会"成功"覆盖已存在的行，UI 就分不清是新增还是重复；
                // 用 OR IGNORE + changes(): 1=新插, 0=已在名单。
                &format!("INSERT OR IGNORE INTO {table} (value, reason) VALUES (?1, ?2)"),
                rusqlite::params![value, r],
            ),
            None => uc.execute(&format!("DELETE FROM {table} WHERE value=?1"), rusqlite::params![value]),
        }
        .map_err(|e| e.to_string())?;
        // 兼容老词库：主库里可能也有同一条，删的时候一并清掉（写不进去就忽略）
        if reason.is_none() {
            if let Ok(c) = self.open_main_writable() {
                let _ = c.execute(&format!("DELETE FROM {table} WHERE value=?1"), rusqlite::params![value]);
            }
        }
        Ok(n)
    }

    /// 添加词组到黑名单。返回 1=新插入, 0=本来就在。
    pub fn add_to_blacklist(&self, value: &str, reason: &str) -> Result<usize, String> {
        self.write_name_list("word_blacklist", value, Some(reason))
    }

    /// 从黑名单移除词组。返回删除行数(0 = 本来就不在)。
    pub fn remove_from_blacklist(&self, value: &str) -> Result<usize, String> {
        self.write_name_list("word_blacklist", value, None)
    }

    /// 添加词组到白名单。返回 1=新插入, 0=本来就在。
    pub fn add_to_whitelist(&self, value: &str, reason: &str) -> Result<usize, String> {
        self.write_name_list("word_whitelist", value, Some(reason))
    }

    /// 从白名单移除词组。返回删除行数(0 = 本来就不在)。
    pub fn remove_from_whitelist(&self, value: &str) -> Result<usize, String> {
        self.write_name_list("word_whitelist", value, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建一个空的临时主库(只需存在,user.db 与之同目录派生),返回 (CikuDb, 临时目录)。
    /// 主库不必有 phrase/pinyin 表——学习路径只写 user.db,本测试不查主库。
    /// 目录名用全局原子计数器:同进程各测试独占目录(并发安全),不会互相清/串行残留。
    /// (曾用 temp_dir+pid:同进程多测试共享 → 并发互删 user.db → 间歇 3/4 假败,2026-09-04 修)
    fn temp_db() -> (CikuDb, std::path::PathBuf) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "prisir_dbtest_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir); // 清掉同目录历史残留
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("ciku.db");
        // 空主库文件(占位,只读打开用)
        Connection::open(&main).unwrap().execute_batch("CREATE TABLE IF NOT EXISTS phrase(jp TEXT,key TEXT,value TEXT,weight INTEGER);").unwrap();
        let db = CikuDb::open(main.to_str().unwrap()).unwrap();
        (db, dir)
    }

    /// 词库窗搜索语义(2026-11-17 从子串改前缀):
    /// 前缀必须命中、计数与分页一致、"词中间"不命中(全表扫 3 秒的代价不值得)。
    #[test]
    fn main_dict_search_is_prefix_only() {
        use rusqlite::Connection;
        let dir = std::env::temp_dir().join(format!("prisir_search_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("ciku.db");
        {
            let c = Connection::open(&main).unwrap();
            c.execute_batch(
                "CREATE TABLE phrase(jp TEXT,key TEXT,value TEXT,weight INTEGER);
                 CREATE INDEX idx_phrase_key ON phrase(key);
                 CREATE INDEX idx_phrase_value ON phrase(value);
                 INSERT INTO phrase VALUES('banquan','banquan','版权',100),
                                         ('banquanjv','banquanjv','版权局',50),
                                         ('banquanfa','banquanfa','版权法',40);",
            )
            .unwrap();
        }
        let db = CikuDb::open(main.to_str().unwrap()).unwrap();

        // 中文前缀
        let rows = db.search_main_dict_page("版权", 10, 0);
        assert_eq!(rows.len(), 3, "版权 前缀应命中 版权/版权局/版权法, 实际 {:?}", rows);
        assert_eq!(db.search_main_dict_count("版权"), 3);
        // 拼音前缀
        let rows = db.search_main_dict_page("banquan", 10, 0);
        assert_eq!(rows.len(), 3);
        // 词中间不命中(改前缀后是有意为之:子串要全表扫,190 万行 3 秒)
        assert!(db.search_main_dict_page("权局", 10, 0).is_empty());
        // 分页与计数一致
        let p1 = db.search_main_dict_page("版权", 2, 0);
        let p2 = db.search_main_dict_page("版权", 2, 2);
        assert_eq!(p1.len(), 2);
        assert_eq!(p2.len(), 1);
    }

    #[test]
    fn learn_pins_word_at_fixed_weight() {
        let (db, _d) = temp_db();
        db.add_user_word("nihao", "你好", 1).unwrap();
        let words = db.user_words_for("nihao");
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].0, "你好");
        // 学成置顶:固定权重写进库,直接读校验
        let all = db.list_user_words();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].2, LEARN_PIN_WEIGHT, "学成词权重必须=LEARN_PIN_WEIGHT(置顶)");
        assert!(LEARN_PIN_WEIGHT > 26000, "LEARN_PIN_WEIGHT 必须大于词库静态 max");
    }

    #[test]
    fn relearn_does_not_accumulate_weight_position_locked() {
        let (db, _d) = temp_db();
        db.add_user_word("nihao", "你好", 1).unwrap();
        // 重复学习多次:weight 不累积(位置锁定)
        for _ in 0..5 {
            db.add_user_word("nihao", "你好", 1).unwrap();
        }
        let all = db.list_user_words();
        assert_eq!(all.len(), 1, "重复学习不应产生多行");
        assert_eq!(all[0].2, LEARN_PIN_WEIGHT, "重复学习 weight 不累积,保持置顶固定值");
    }

    #[test]
    fn multiple_learned_words_ordered_by_seq() {
        let (db, _d) = temp_db();
        db.add_user_word("ni", "你", 1).unwrap();
        db.add_user_word("ni", "泥", 1).unwrap();
        db.add_user_word("ni", "尼", 1).unwrap();
        let words = db.user_words_for("ni");
        let vals: Vec<&str> = words.iter().map(|(v, _)| v.as_str()).collect();
        assert_eq!(vals, vec!["你", "泥", "尼"], "学成词按学习先后(seq)稳定排序");
    }

    #[test]
    fn remove_and_clear_user_words() {
        let (db, _d) = temp_db();
        db.add_user_word("nihao", "你好", 1).unwrap();
        db.add_user_word("nihao", "泥嚎", 1).unwrap();
        assert_eq!(db.user_words_for("nihao").len(), 2);
        let n = db.remove_user_word("nihao", "泥嚎").unwrap();
        assert_eq!(n, 1);
        let rest = db.user_words_for("nihao");
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].0, "你好");
        db.clear_user_words().unwrap();
        assert_eq!(db.user_words_for("nihao").len(), 0, "清空后无学成词");
    }
}
