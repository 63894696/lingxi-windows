//! C ABI 导出(为 Chromium FFI 集成 / Python ctypes 验证用,对齐 prisir_asr FFI 形态)
//!
//! 用法(C 侧):
//!   void* h = prisir_ime_load(".../ciku.db");
//!   char* json = prisir_ime_query(h, "nihao");            // [{"word":"你好","weight":123},...]
//!   char* sent = prisir_ime_smart_sentence(h, "nihaoshijie");
//!   prisir_ime_free_string(json); prisir_ime_free_string(sent);
//!   prisir_ime_free(h);

use crate::engine::ImeEngine;
use std::ffi::{c_char, c_void, CStr, CString};

fn cstr(ptr: *const c_char) -> Option<&'static str> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().ok()
}

fn to_c_string(s: String) -> *mut c_char {
    CString::new(s).map(CString::into_raw).unwrap_or(std::ptr::null_mut())
}

fn get<'a>(handle: *mut c_void) -> Option<&'a ImeEngine> {
    if handle.is_null() {
        return None;
    }
    Some(unsafe { &*(handle as *const ImeEngine) })
}

/// ABI 版本号(monotonic)。三端(Windows TSF / Linux fcitx5 / Android JNI)都按它对齐:
/// 只要导出签名或语义有破坏性变更就 +1,老外壳加载新引擎(或反之)可据此报错而不是崩。
///   v1: 2026-11-17 起(37+ 导出,含 P0/P1/P2 与用户词典/黑白名单)
pub const ABI_VERSION: i32 = 1;

/// 返回 ABI 版本。外壳启动时打印/校验用。
#[no_mangle]
pub extern "C" fn prisir_ime_abi_version() -> i32 {
    ABI_VERSION
}

/// 加载词库,返回 opaque 句柄(失败返回 null)。build_index!=0 时加载内存 Trie 索引:
/// 优先读 `<db>.idx` 持久化缓存(指纹对上则秒开),否则首次构建并落盘,之后启动直接加载。
#[no_mangle]
pub extern "C" fn prisir_ime_load(db_path: *const c_char, build_index: i32) -> *mut c_void {
    let Some(path) = cstr(db_path) else {
        return std::ptr::null_mut();
    };
    let mut engine = match ImeEngine::new(path) {
        Ok(e) => e,
        Err(_) => return std::ptr::null_mut(),
    };
    // 模糊音规则:读配置文件(2026-09-04 模糊音设置功能,替代硬编码)。
    // 规则见 engine.rs fuzzy_map。文件不存在=默认三组平翘舌;存在但空=关闭模糊音。
    engine.set_fuzzy_rules(load_fuzzy_rules(path));
    // P2 简繁输出模式(2026-11-16,双向 2026-11-17):同目录 `traditional.txt`。默认原样。
    engine.set_script(crate::engine::ScriptMode::from_i32(load_script_mode(path)));
    if build_index != 0 {
        let (_ok, src) = engine.load_or_build_index(path);
        eprintln!("[prisir_ime] index source: {src}");
    }
    Box::into_raw(Box::new(engine)) as *mut c_void
}

/// 读模糊音配置(2026-09-04)。配置文件 = 词库同目录的 `fuzzy.txt`
/// (ciku.db → fuzzy.txt),每行一个规则名,`#` 开头为注释。
///   - 文件不存在 → 默认三组平翘舌 ["z_zh","c_ch","s_sh"](对齐外挂 ime_config)
///   - 文件存在但无任何有效行 → 关闭模糊音(返空 Vec)
///   - 否则 → 文件里列出的规则(非法规则名会被 fuzzy_map 忽略,不致命)
/// 引擎句柄 OnceLock 只建一次,故启动时读一次即可,无需热重载。
/// 读简繁输出模式。配置文件 = 词库同目录的 `traditional.txt`（文件名沿用旧名，
/// 因为最早只支持简→繁；现在同时支持双向）：
///   - 文件不存在 → 0（原样）
///   - `1` / `true` / `on` / `s2t` → 1（简→繁）
///   - `2` / `t2s`                → 2（繁→简）
///   - 其它                        → 0
fn load_script_mode(db_path: &str) -> i32 {
    let cfg = std::path::Path::new(db_path).with_file_name("traditional.txt");
    let Ok(text) = std::fs::read_to_string(&cfg) else {
        return 0;
    };
    let v = text.trim().to_ascii_lowercase();
    let mode = match v.as_str() {
        "1" | "true" | "on" | "yes" | "s2t" | "trad" | "traditional" => 1,
        "2" | "t2s" | "simp" | "simplified" => 2,
        _ => 0,
    };
    eprintln!("[prisir_ime] script mode = {mode} (from {:?})", cfg);
    mode
}

fn load_fuzzy_rules(db_path: &str) -> Vec<String> {
    let cfg = std::path::Path::new(db_path).with_file_name("fuzzy.txt");
    let Ok(text) = std::fs::read_to_string(&cfg) else {
        // 文件不存在 → 默认三组平翘舌。
        return vec!["z_zh".into(), "c_ch".into(), "s_sh".into()];
    };
    let mut rules = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        rules.push(t.to_string());
    }
    eprintln!("[prisir_ime] fuzzy rules from {:?}: {:?}", cfg, rules);
    rules
}

/// 候选查询,返回 JSON 数组 [{"word":"...","weight":N},...](调用方须 free_string)
#[no_mangle]
pub extern "C" fn prisir_ime_query(handle: *mut c_void, input: *const c_char) -> *mut c_char {
    let Some(inp) = cstr(input) else {
        return std::ptr::null_mut();
    };
    let Some(engine) = get(handle) else {
        return std::ptr::null_mut();
    };
    let cands = engine.query(inp);
    let arr: Vec<serde_json::Value> = cands
        .iter()
        .map(|(w, wt)| serde_json::json!({"word": w, "weight": wt}))
        .collect();
    to_c_string(serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string()))
}

/// 九宫格候选查询：digits 为 2-9 数字串。返回 [{"word","weight"}]。
#[no_mangle]
pub extern "C" fn prisir_ime_query_t9(handle: *mut c_void, digits: *const c_char) -> *mut c_char {
    let Some(d) = cstr(digits) else {
        return to_c_string("[]".to_string());
    };
    match get(handle) {
        Some(engine) => {
            let cands = engine.query_t9(d);
            let arr: Vec<serde_json::Value> = cands
                .iter()
                .map(|(w, wt)| serde_json::json!({"word": w, "weight": wt}))
                .collect();
            to_c_string(serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string()))
        }
        None => to_c_string("[]".to_string()),
    }
}

/// 五笔候选查询(wubi86),返回 JSON 数组 [{"word":"...","weight":N},...](调用方须 free_string)
#[no_mangle]
pub extern "C" fn prisir_ime_query_wubi(handle: *mut c_void, input: *const c_char) -> *mut c_char {
    let Some(inp) = cstr(input) else {
        return std::ptr::null_mut();
    };
    let Some(engine) = get(handle) else {
        return std::ptr::null_mut();
    };
    let cands = engine.query_wubi(inp);
    let arr: Vec<serde_json::Value> = cands
        .iter()
        .map(|(w, wt)| serde_json::json!({"word": w, "weight": wt}))
        .collect();
    to_c_string(serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string()))
}

/// 整句智能首选,返回拼接整句(无路径返回空串;调用方须 free_string)
#[no_mangle]
pub extern "C" fn prisir_ime_smart_sentence(handle: *mut c_void, input: *const c_char) -> *mut c_char {
    let Some(inp) = cstr(input) else {
        return std::ptr::null_mut();
    };
    let Some(engine) = get(handle) else {
        return std::ptr::null_mut();
    };
    to_c_string(engine.smart_sentence(inp).unwrap_or_default())
}

/// P1 整句候选：只返回**多词整句**（`nwords >= min_words`）的句子，JSON 字符串数组。
///
/// 给 C 侧（fcitx5 addon）用：不需要 score/nwords，只要句字串。
/// 过滤在 Rust 侧做，C 侧不用解 nwords。
#[no_mangle]
pub extern "C" fn prisir_ime_sentences(
    handle: *mut c_void,
    input: *const c_char,
    k: i32,
    min_words: i32,
) -> *mut c_char {
    let Some(inp) = cstr(input) else {
        return to_c_string("[]".to_string());
    };
    let Some(engine) = get(handle) else {
        return to_c_string("[]".to_string());
    };
    let minw = min_words.max(1) as usize;
    let arr: Vec<String> = engine
        .decode(inp, (k.max(1) as usize) * 3)
        .into_iter()
        .filter(|d| d.nwords >= minw)
        .take(k.max(1) as usize)
        .map(|d| d.sentence)
        .collect();
    to_c_string(serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string()))
}

/// P1 单条整句（仅多词整句），无可用时返回空串。
/// 给 Windows TSF 用：原有 `prisir_ime_smart_sentence` 会在只有 1 个词时
/// 把用户原有首选覆盖掉（打 `sj` 会用 时间 顶掉 设计/世界），故新增此接口。
#[no_mangle]
pub extern "C" fn prisir_ime_smart_sentence_min(
    handle: *mut c_void,
    input: *const c_char,
    min_words: i32,
) -> *mut c_char {
    let Some(inp) = cstr(input) else {
        return to_c_string(String::new());
    };
    let Some(engine) = get(handle) else {
        return to_c_string(String::new());
    };
    let minw = min_words.max(1) as usize;
    let decoded = engine.decode(inp, 8);
    // 2026-11-21: 若 lattice 已有「整词覆盖」(nwords==1，如 菜单/出来)，
    // 不要再返回多词拼接（才但/处来）给 Win 空格覆盖首选 —— 那正是
    // smart_sentence_min 想避免的另一面（原先只防了 nwords==1 去顶 sj）。
    if minw >= 2 && decoded.iter().any(|d| d.nwords == 1) {
        return to_c_string(String::new());
    }
    let best = decoded
        .into_iter()
        .find(|d| d.nwords >= minw)
        .map(|d| d.sentence)
        .unwrap_or_default();
    to_c_string(best)
}

/// P1 整句解码（top-K）→ JSON `[{"sentence":"..","score":1.2,"nwords":3},...]`。
///
/// `nwords` 让调用方区分“这是多词整句”还是“只是一个词”：
/// 单词语句不要拿去抢候选首位（否则打 `sj` 会把原有首选覆盖掉）。
#[no_mangle]
pub extern "C" fn prisir_ime_decode(
    handle: *mut c_void,
    input: *const c_char,
    k: i32,
) -> *mut c_char {
    let Some(inp) = cstr(input) else {
        return to_c_string("[]".to_string());
    };
    let Some(engine) = get(handle) else {
        return to_c_string("[]".to_string());
    };
    let items = engine.decode(inp, k.max(1) as usize);
    let arr: Vec<serde_json::Value> = items
        .iter()
        .map(|d| {
            serde_json::json!({
                "sentence": d.sentence,
                "score": d.score,
                "nwords": d.nwords,
            })
        })
        .collect();
    to_c_string(serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string()))
}

/// 学习用户选择(更新词频)
#[no_mangle]
pub extern "C" fn prisir_ime_learn(handle: *mut c_void, input: *const c_char, selected: *const c_char) {
    let (Some(inp), Some(sel)) = (cstr(input), cstr(selected)) else {
        return;
    };
    if let Some(engine) = get(handle) {
        engine.learn(inp, sel);
    }
}

/// 释放 query/smart_sentence 返回的字符串
#[no_mangle]
pub extern "C" fn prisir_ime_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe { drop(CString::from_raw(s)) };
    }
}

/// 释放引擎句柄
#[no_mangle]
pub extern "C" fn prisir_ime_free(handle: *mut c_void) {
    if !handle.is_null() {
        unsafe { drop(Box::from_raw(handle as *mut ImeEngine)) };
    }
}

// ============================================================
// 学习词库管理(Step 2 词库管理窗口用,2026-09-04)
// 走独立 user.db(不碰只读主库)。user.db 打开失败时这些方法静默无效,
// 由调用方按返回(JSON 空数组 / false)处理。字符串一律 UTF-8。
// ============================================================

/// 列出全部学习词 → JSON 数组 [{"key":"nihao","value":"你好","weight":100000,"seq":2,"pos":0},...]
/// 按 seq(学习先后)升序。`pos>0` = 手工造词指定的候选位(快捷造词),0 = 自动学成。
/// 调用方须 `prisir_ime_free_string` 释放。
#[no_mangle]
pub extern "C" fn prisir_ime_user_list(handle: *mut c_void) -> *mut c_char {
    let Some(engine) = get(handle) else {
        return to_c_string("[]".to_string());
    };
    let rows = engine.db.list_user_words_with_pos();
    let arr: Vec<serde_json::Value> = rows
        .iter()
        .map(|(k, v, w, s, p)| serde_json::json!({"key": k, "value": v, "weight": w, "seq": s, "pos": p}))
        .collect();
    to_c_string(serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string()))
}

/// 手工造词:按 `pos`(1 起的候选位)写入 user_word。返回 1=成功, 0=失败。
///
/// 2026-11-24 新增(借鉴影子输入法「快捷造词」)。与 `user_add` 的区别:
///   - `user_add`  → 自动学成语义(seq>0,固定置顶)
///   - 本函数      → 手工语义(seq=0,pos 决定插在第几位)
#[no_mangle]
pub extern "C" fn prisir_ime_user_add_pos(
    handle: *mut c_void,
    pinyin: *const c_char,
    word: *const c_char,
    pos: i64,
) -> i32 {
    let (Some(p), Some(w)) = (cstr(pinyin), cstr(word)) else {
        return 0;
    };
    match get(handle) {
        Some(engine) if engine.user_add_pos(p, w, pos).is_ok() => 1,
        _ => 0,
    }
}

/// 中文词 → 自动编码(简拼)。"北京大学" → "bjdx"。失败返 null。
/// 快捷造词窗口 / Agent 批量入库用:用户只给中文,编码由引擎算。
#[no_mangle]
pub extern "C" fn prisir_ime_auto_code(handle: *mut c_void, word: *const c_char) -> *mut c_char {
    let Some(w) = cstr(word) else {
        return std::ptr::null_mut();
    };
    match get(handle).and_then(|e| e.auto_code(w)) {
        Some(code) if !code.is_empty() => to_c_string(code),
        _ => std::ptr::null_mut(),
    }
}

/// 中文词 → **全拼**。"北京大学" → "beijingdaxue";算不出来 → null。
///
/// 2026-11-25:造词写库时优先用它当主 key(简拼由引擎自动补别名行),
/// 这样全拼、简拼、混拼三种打法都能出词。`auto_code` 仍保留给窗口显示用。
#[no_mangle]
pub extern "C" fn prisir_ime_word_pinyin(handle: *mut c_void, word: *const c_char) -> *mut c_char {
    let Some(w) = cstr(word) else {
        return std::ptr::null_mut();
    };
    match get(handle).and_then(|e| e.word_pinyin(w)) {
        Some(code) if !code.is_empty() => to_c_string(code),
        _ => std::ptr::null_mut(),
    }
}

/// 手动加词(走学成置顶固定权重)。返回 1=成功, 0=失败。
#[no_mangle]
pub extern "C" fn prisir_ime_user_add(handle: *mut c_void, pinyin: *const c_char, word: *const c_char) -> i32 {
    let (Some(p), Some(w)) = (cstr(pinyin), cstr(word)) else {
        return 0;
    };
    match get(handle) {
        Some(engine) if engine.user_add(p, w).is_ok() => 1,
        _ => 0,
    }
}

/// 删除某条学习词。返回删除行数(0=不存在/失败)。
#[no_mangle]
pub extern "C" fn prisir_ime_user_remove(handle: *mut c_void, pinyin: *const c_char, word: *const c_char) -> i32 {
    let (Some(p), Some(w)) = (cstr(pinyin), cstr(word)) else {
        return 0;
    };
    match get(handle) {
        Some(engine) => engine.user_remove(p, w).unwrap_or(0) as i32,
        None => 0,
    }
}

/// 按**词本身**删除学习行(不需要编码)。返回删除行数。
///
/// 2026-11-25:用户能看见候选里的垃圾词,但不记得当初的输入串 →
/// 清理时必须能只凭词删除。GUI「学习词」行删除与 `--del-word` 都走这里。
#[no_mangle]
pub extern "C" fn prisir_ime_user_remove_value(handle: *mut c_void, word: *const c_char) -> i32 {
    let Some(w) = cstr(word) else { return 0; };
    match get(handle) {
        Some(engine) => engine.user_remove_by_value(w).unwrap_or(0) as i32,
        None => 0,
    }
}

/// 在用户词层搜索(子串;key 或 value 命中都算)。空串 = 最近学成的 50 条。
/// 返回 JSON 数组 [{"key":..,"value":..,"weight":N},...]。调用方须 free_string。
#[no_mangle]
pub extern "C" fn prisir_ime_user_search(
    handle: *mut c_void,
    term: *const c_char,
    limit: i32,
) -> *mut c_char {
    let Some(t) = cstr(term) else { return to_c_string("[]".to_string()); };
    let lim = if limit <= 0 { 50 } else { limit as usize };
    let Some(engine) = get(handle) else {
        return to_c_string("[]".to_string());
    };
    let rows = engine.user_search(t, lim);
    let arr: Vec<serde_json::Value> = rows
        .iter()
        .map(|(k, v, w)| serde_json::json!({"key": k, "value": v, "weight": w}))
        .collect();
    to_c_string(serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string()))
}

/// 清空全部学习记录。返回 1=成功, 0=失败。
#[no_mangle]
pub extern "C" fn prisir_ime_user_clear(handle: *mut c_void) -> i32 {
    match get(handle) {
        Some(engine) if engine.user_clear().is_ok() => 1,
        _ => 0,
    }
}

/// 搜索主词库(只读)→ JSON 数组 [{"key":"..","value":"..","weight":N,"source":"phrase|pinyin"},...]
/// 词库管理窗口「全部词库」查询用。主库只读,只能查不能改。调用方须 `prisir_ime_free_string` 释放。
#[no_mangle]
pub extern "C" fn prisir_ime_dict_search(handle: *mut c_void, term: *const c_char) -> *mut c_char {
    let Some(t) = cstr(term) else {
        return to_c_string("[]".to_string());
    };
    let Some(engine) = get(handle) else {
        return to_c_string("[]".to_string());
    };
    let rows = engine.search_main_dict(t, 200);
    let arr: Vec<serde_json::Value> = rows
        .iter()
        .map(|(k, v, w, s)| serde_json::json!({"key": k, "value": v, "weight": w, "source": s}))
        .collect();
    to_c_string(serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string()))
}

/// 分页搜索主词库(2026-09-04 灵犀式分页)→ 同 dict_search 的 JSON 数组。
/// offset 跳过前 N 条;空 term 返回全表 top(按权重)。调用方须 `prisir_ime_free_string` 释放。
#[no_mangle]
pub extern "C" fn prisir_ime_dict_search_page(
    handle: *mut c_void,
    term: *const c_char,
    limit: i32,
    offset: i32,
) -> *mut c_char {
    let Some(t) = cstr(term) else {
        return to_c_string("[]".to_string());
    };
    let Some(engine) = get(handle) else {
        return to_c_string("[]".to_string());
    };
    let lim = if limit <= 0 { 200 } else { limit as usize };
    let off = if offset < 0 { 0 } else { offset as usize };
    let rows = engine.search_main_dict_page(t, lim, off);
    let arr: Vec<serde_json::Value> = rows
        .iter()
        .map(|(k, v, w, s)| serde_json::json!({"key": k, "value": v, "weight": w, "source": s}))
        .collect();
    to_c_string(serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string()))
}

/// 主库匹配总条数(分页「共 N 条」)。空 term = 全表总数。
#[no_mangle]
pub extern "C" fn prisir_ime_dict_count(handle: *mut c_void, term: *const c_char) -> i64 {
    let Some(t) = cstr(term) else {
        return 0;
    };
    match get(handle) {
        Some(engine) => engine.main_dict_count(t) as i64,
        None => 0,
    }
}

// ============================================================
// 主库可写(2026-09-04,恢复外挂式删/加/改权重,用户拍板)
// 走独立短时可写连接(见 db.rs),运行时只读引擎不受影响。
// ============================================================

/// 主库删除词条。返回删除行数(0=不存在/失败)。
#[no_mangle]
pub extern "C" fn prisir_ime_main_delete(handle: *mut c_void, pinyin: *const c_char, word: *const c_char) -> i32 {
    let (Some(p), Some(w)) = (cstr(pinyin), cstr(word)) else {
        return 0;
    };
    match get(handle) {
        Some(engine) => engine.main_delete_word(p, w).unwrap_or(0) as i32,
        None => 0,
    }
}

/// 由中文词反推全拼（逐字查单字表）。返回全拼字符串；算不出返回 NULL。
/// 用途：用户加词只需输入中文，拼音交给引擎（三端共用）。
#[no_mangle]
pub extern "C" fn prisir_ime_pinyin_for(handle: *mut c_void, word: *const c_char) -> *mut c_char {
    let Some(w) = cstr(word) else {
        return std::ptr::null_mut();
    };
    match get(handle).and_then(|e| e.db.pinyin_for_word(w)) {
        Some(k) => to_c_string(k),
        None => std::ptr::null_mut(),
    }
}

/// 主库 upsert 词条(加词/改权重)。返回 1=成功, 0=失败。
#[no_mangle]
pub extern "C" fn prisir_ime_main_upsert(handle: *mut c_void, pinyin: *const c_char, word: *const c_char, weight: i64) -> i32 {
    let (Some(p), Some(w)) = (cstr(pinyin), cstr(word)) else {
        return 0;
    };
    match get(handle) {
        Some(engine) => match engine.main_upsert_word(p, w, weight) {
            Ok(true) => 1,
            _ => 0,
        },
        None => 0,
    }
}

/// 同 key 主库 phrase 最高词频(改权重参考)。返回权重(0=无)。
#[no_mangle]
pub extern "C" fn prisir_ime_main_max_weight(handle: *mut c_void, pinyin: *const c_char) -> i64 {
    let Some(p) = cstr(pinyin) else {
        return 0;
    };
    match get(handle) {
        Some(engine) => engine.main_max_weight(p),
        None => 0,
    }
}

// ============================================================
// 黑白名单管理 C ABI (2026-11-15,对齐 Python professional_corrector)
// 供词库管理 UI 调用:增删改查黑/白名单。
// ============================================================

/// 添加词组到黑名单。返回 1=成功, 0=失败。
#[no_mangle]
pub extern "C" fn prisir_ime_add_blacklist(handle: *mut c_void, value: *const c_char, reason: *const c_char) -> i32 {
    let (Some(v), Some(r)) = (cstr(value), cstr(reason)) else {
        return 0;
    };
    match get(handle) {
        Some(engine) => match engine.db.add_to_blacklist(v, r) {
            Ok(n) => n as i32, // 1=新插入 0=已在名单
            _ => 0,
        },
        None => 0,
    }
}

/// 从黑名单移除词组。返回 1=成功, 0=失败。
#[no_mangle]
pub extern "C" fn prisir_ime_remove_blacklist(handle: *mut c_void, value: *const c_char) -> i32 {
    let Some(v) = cstr(value) else {
        return 0;
    };
    match get(handle) {
        Some(engine) => match engine.db.remove_from_blacklist(v) {
            Ok(n) => n as i32, // 删除行数(0=本来就不在)
            _ => 0,
        },
        None => 0,
    }
}

/// 获取全部黑名单 → JSON 数组 [{"value":"..","reason":".."},...]。
#[no_mangle]
pub extern "C" fn prisir_ime_get_blacklist(handle: *mut c_void) -> *mut c_char {
    match get(handle) {
        Some(engine) => {
            let rows = engine.db.get_blacklist();
            let arr: Vec<serde_json::Value> = rows
                .iter()
                .map(|(v, r)| serde_json::json!({"value": v, "reason": r}))
                .collect();
            to_c_string(serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string()))
        }
        None => to_c_string("[]".to_string()),
    }
}

/// 添加词组到白名单。返回 1=成功, 0=失败。
#[no_mangle]
pub extern "C" fn prisir_ime_add_whitelist(handle: *mut c_void, value: *const c_char, reason: *const c_char) -> i32 {
    let (Some(v), Some(r)) = (cstr(value), cstr(reason)) else {
        return 0;
    };
    match get(handle) {
        Some(engine) => match engine.db.add_to_whitelist(v, r) {
            Ok(n) => n as i32, // 1=新插入 0=已在名单
            _ => 0,
        },
        None => 0,
    }
}

/// 从白名单移除词组。返回 1=成功, 0=失败。
#[no_mangle]
pub extern "C" fn prisir_ime_remove_whitelist(handle: *mut c_void, value: *const c_char) -> i32 {
    let Some(v) = cstr(value) else {
        return 0;
    };
    match get(handle) {
        Some(engine) => match engine.db.remove_from_whitelist(v) {
            Ok(n) => n as i32, // 删除行数(0=本来就不在)
            _ => 0,
        },
        None => 0,
    }
}

/// P0 中英文自动判定(2026-11-16)。
/// 返回:0=空 1=拼音 2=英文 3=单字母存疑。空句柄/空指针返回 0。
#[no_mangle]
pub extern "C" fn prisir_ime_classify(handle: *mut c_void, input: *const c_char) -> i32 {
    let Some(inp) = cstr(input) else {
        return 0;
    };
    match get(handle) {
        Some(engine) => engine.classify_input(inp).as_u32() as i32,
        None => 0,
    }
}

/// 同 `prisir_ime_classify`,但允许调用方直接给“是否有词库精确命中”的结论。
///
/// `exact_flag`:
///   -  `< 0` → 引擎自己查(等价 `prisir_ime_classify`,推荐)
///   -  `== 0` → 调用方认定无精确命中(例:纯英文)
///   -  `> 0` → 调用方认定有精确命中(例:已知是混拼词)
///
/// ⚠️ 这里的 flag 是“精确命中”而非“候选数”:引擎 query() 有模糊音/贪心兼底,
/// 候选数恒 > 0,拿它判会把 hello/world 全判成拼音。
#[no_mangle]
pub extern "C" fn prisir_ime_classify_with(
    handle: *mut c_void,
    input: *const c_char,
    exact_flag: i32,
) -> i32 {
    let Some(inp) = cstr(input) else {
        return 0;
    };
    match get(handle) {
        Some(engine) => {
            let kind = if exact_flag < 0 {
                engine.classify_input(inp)
            } else {
                engine.classify_input_with(inp, exact_flag > 0)
            };
            kind.as_u32() as i32
        }
        None => 0,
    }
}

/// P0 智能标点(2026-11-16)。`ch` 为 Unicode 码点。
/// `reset != 0` 先归位引号配对状态(换输入框/上屏/回车时传 1)。
/// 返UTF-8 中文标点;无映射返回 NULL——调用方按原键处理。
#[no_mangle]
pub extern "C" fn prisir_ime_punct(handle: *mut c_void, ch: u32, reset: i32) -> *mut c_char {
    let Some(c) = char::from_u32(ch) else {
        return std::ptr::null_mut();
    };
    match get(handle) {
        Some(engine) => match engine.punct_map(c, reset != 0) {
            Some(s) => to_c_string(s),
            None => std::ptr::null_mut(),
        },
        None => std::ptr::null_mut(),
    }
}

/// P2 简繁：设置输出模式。0=原样, 1=简→繁, 2=繁→简。
#[no_mangle]
pub extern "C" fn prisir_ime_set_script(handle: *mut c_void, mode: i32) {
    if let Some(engine) = get(handle) {
        engine.set_script(crate::engine::ScriptMode::from_i32(mode));
    }
}

/// P2 简繁：当前模式(0/1/2)。无句柄返回 0。
#[no_mangle]
pub extern "C" fn prisir_ime_script(handle: *mut c_void) -> i32 {
    match get(handle) {
        Some(engine) => engine.script().as_i32(),
        None => 0,
    }
}

/// P2 简繁：循环切换 原样 → 简→繁 → 繁→简 → 原样，返回新模式。
/// 给 UI 按钮用，避免两端各自实现轮换逻辑。
#[no_mangle]
pub extern "C" fn prisir_ime_cycle_script(handle: *mut c_void) -> i32 {
    match get(handle) {
        Some(engine) => engine.cycle_script().as_i32(),
        None => 0,
    }
}

/// P2 简繁：按当前模式转换。原样模式直接返回原文（调用方无需判断模式）。
#[no_mangle]
pub extern "C" fn prisir_ime_to_script(handle: *mut c_void, text: *const c_char) -> *mut c_char {
    let Some(t) = cstr(text) else {
        return to_c_string(String::new());
    };
    match get(handle) {
        Some(engine) => to_c_string(engine.to_script(t)),
        None => to_c_string(t.to_string()),
    }
}

// ---------- 旧接口（保留兼容：true/false 语义 = 简→繁/原样） ----------

#[no_mangle]
pub extern "C" fn prisir_ime_set_trad(handle: *mut c_void, on: i32) {
    if let Some(engine) = get(handle) {
        engine.set_trad(on != 0);
    }
}

#[no_mangle]
pub extern "C" fn prisir_ime_trad_enabled(handle: *mut c_void) -> i32 {
    match get(handle) {
        Some(engine) => i32::from(engine.trad_enabled()),
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn prisir_ime_to_trad(handle: *mut c_void, text: *const c_char) -> *mut c_char {
    prisir_ime_to_script(handle, text)
}

/// 单独归位标点状态(不回传字符)。
#[no_mangle]
pub extern "C" fn prisir_ime_punct_reset(handle: *mut c_void) {
    if let Some(engine) = get(handle) {
        engine.punct_reset();
    }
}

/// 获取全部白名单 → JSON 数组 [{"value":"..","reason":".."},...]。
#[no_mangle]
pub extern "C" fn prisir_ime_get_whitelist(handle: *mut c_void) -> *mut c_char {
    match get(handle) {
        Some(engine) => {
            let rows = engine.db.get_whitelist();
            let arr: Vec<serde_json::Value> = rows
                .iter()
                .map(|(v, r)| serde_json::json!({"value": v, "reason": r}))
                .collect();
            to_c_string(serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string()))
        }
        None => to_c_string("[]".to_string()),
    }
}
