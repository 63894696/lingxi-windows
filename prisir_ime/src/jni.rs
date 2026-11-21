//! JNI 导出(Android),叠加于现有 C ABI(ffi.rs)之上,不动其形态。
//! Java 侧: com.lingxi.ime.Engine 声明 native 方法,System.loadLibrary("prisir_ime")。
//! 内部直接复用 engine::ImeEngine 的 query/smart_sentence/learn,不重复逻辑。

use crate::engine::ImeEngine;

// 裸 dlopen/dlsym 声明（只为 gpen 探针；不让整个 crate 依赖 libc）
// ⚠️ 手写功能已于 2026-11-19 下线：下面几个 nativeGpen* 导出在产品里已无调用方，
//    作为逆向成果保留（可执行的探针源码见 ~/oi_enhancements/gpen_probe）。
extern "C" {
    fn dlopen(filename: *const u8, flag: i32) -> *mut std::ffi::c_void;
    fn dlsym(handle: *mut std::ffi::c_void, symbol: *const u8) -> *mut std::ffi::c_void;
    fn dlerror() -> *mut u8;
}
use jni::objects::{JClass, JObject, JString};
use jni::sys::{jboolean, jint, jlong, jstring};
use jni::JNIEnv;

/// 句柄: jlong 即 *mut ImeEngine。
fn engine_of(handle: jlong) -> Option<&'static ImeEngine> {
    if handle == 0 {
        return None;
    }
    Some(unsafe { &*(handle as *const ImeEngine) })
}

fn jstr(env: &mut JNIEnv, s: &JString) -> String {
    env.get_string(s)
        .map(|v| v.into())
        .unwrap_or_else(|_| String::new())
}

fn out(env: &mut JNIEnv, s: String) -> jstring {
    env.new_string(s)
        .map(|v| v.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// 加载词库,返回引擎句柄(0=失败)。buildIndex!=0 时加载/构建内存 Trie 索引。
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeLoad(
    mut env: JNIEnv,
    _cls: JClass,
    db_path: JString,
    build_index: jboolean,
) -> jlong {
    let path = jstr(&mut env, &db_path);
    if path.is_empty() {
        return 0;
    }
    let mut engine = match ImeEngine::new(&path) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    if build_index != 0 {
        let (_ok, src) = engine.load_or_build_index(&path);
        eprintln!("[prisir_ime] index source: {src}");
    }
    Box::into_raw(Box::new(engine)) as jlong
}

/// **诊断用**：尝试 dlopen 外部手写引擎（例如搜狗 20.20 的 libgpen_handwriter.so），
/// 报告能否加载 + 关键符号是否存在 + HWR_Version 的原始返回。
///
/// 目的：回答"能否把搜狗的模型加载拆出来复用"。不参与正式功能，只在状态页点一下。
/// 用裸 dlopen 声明，避免为一个探针引入 libc 依赖。
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeGpenProbe(
    mut env: JNIEnv,
    _cls: JClass,
    path: JString,
) -> jstring {
    let p = jstr(&mut env, &path);
    if p.is_empty() {
        return out(&mut env, "path 为空".to_string());
    }
    let mut report = String::new();
    let Ok(cpath) = std::ffi::CString::new(p.clone()) else {
        return out(&mut env, "path 含非法字符".to_string());
    };
    const RTLD_NOW: i32 = 2;
    const RTLD_GLOBAL: i32 = 0x00100;
    let h = unsafe { dlopen(cpath.as_ptr(), RTLD_NOW | RTLD_GLOBAL) };
    if h.is_null() {
        let e = unsafe {
            let ep = dlerror();
            if ep.is_null() { "未知错误".to_string() } else { std::ffi::CStr::from_ptr(ep).to_string_lossy().into_owned() }
        };
        return out(&mut env, format!("dlopen 失败: {e}"));
    }
    report.push_str("dlopen: OK
");
    for sym in ["HWR_Version", "HWR_Init", "HWR_Configure", "HWR_RealRecognize",
                "HWR_GetAllRegResult", "HWR_Beautify", "HWR_Load_Param", "HWR_Clear"] {
        let Ok(cs) = std::ffi::CString::new(sym) else { continue };
        let f = unsafe { dlsym(h, cs.as_ptr()) };
        report.push_str(&format!("  {sym}: {}
", if f.is_null() { "缺失" } else { "有" }));
    }
    // HWR_Version 只有 12 字节，极可能是 int(void) 或 const char*(void)。两种都试，只做打印。
    let Ok(cs) = std::ffi::CString::new("HWR_Version") else {
        return out(&mut env, report);
    };
    let f = unsafe { dlsym(h, cs.as_ptr()) };
    if !f.is_null() {
        let as_int: extern "C" fn() -> i32 = unsafe { std::mem::transmute(f) };
        let v = as_int();
        report.push_str(&format!("  HWR_Version() as int = {v}
"));
    }
    out(&mut env, report)
}

/// 候选查询,返回 JSON 数组 [{"word":"...","weight":N},...]。
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeQuery(
    mut env: JNIEnv,
    _cls: JClass,
    handle: jlong,
    input: JString,
) -> jstring {
    let Some(engine) = engine_of(handle) else {
        return out(&mut env, "[]".to_string());
    };
    let inp = jstr(&mut env, &input);
    let cands = engine.query(&inp);
    let arr: Vec<serde_json::Value> = cands
        .iter()
        .map(|(w, wt)| serde_json::json!({"word": w, "weight": wt}))
        .collect();
    out(&mut env, serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string()))
}

/// **诊断用**:同一输入在 mmap 路径与 SQLite 路径下的耗时/首屏候选对比(JSON)。
///
/// Android 端把候选查询搬到 mmap 索引上,必须证明"结果与 SQLite 一致"。
/// 引擎侧临时摘掉/装回内存索引各查一次,上层比对首屏 10 个候选是否逐条相同。
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeQueryProfile(
    mut env: JNIEnv,
    _cls: JClass,
    handle: jlong,
    input: JString,
) -> jstring {
    if handle == 0 {
        return out(&mut env, "{}".to_string());
    }
    let inp = jstr(&mut env, &input);
    // 与 nativeLoadMmap 同源:句柄由 nativeLoad 创建,这里需要可变借用以切换内存索引。
    let e = unsafe { &mut *(handle as *mut ImeEngine) };
    out(&mut env, e.query_profile(&inp))
}

/// 整句智能首选(DP/Viterbi),无路径返回空串。
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeSmartSentence(
    mut env: JNIEnv,
    _cls: JClass,
    handle: jlong,
    input: JString,
) -> jstring {
    let Some(engine) = engine_of(handle) else {
        return out(&mut env, String::new());
    };
    let inp = jstr(&mut env, &input);
    out(&mut env, engine.smart_sentence(&inp).unwrap_or_default())
}

/// 学习用户选择(更新词频)。
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeLearn(
    mut env: JNIEnv,
    _cls: JClass,
    handle: jlong,
    input: JString,
    selected: JString,
) {
    if let Some(engine) = engine_of(handle) {
        let inp = jstr(&mut env, &input);
        let sel = jstr(&mut env, &selected);
        engine.learn(&inp, &sel);
    }
}

/// **诊断用（第二阶段）**：读 `HWR_GetFunctions()` 返回的函数表，并用 4 种参数顺序试 `HWR_Init`。
///
/// 依据（对方解反汇编，见 Obsidian《搜狗手写引擎gpen-ABI逆向阶段报告》）：
///   - gpen 的 13 个导出都是 thunk；真正 ABI 是 `HWR_GetFunctions()` 返回的**函数指针表**
///   - 调用方 `libsogouime.so` 里那张表的槽位签名已确认：
///       +0x00 f(ptr) / +0x08 f(int,int,int,bool,bool) / +0x18 f(ptr,int)
///       +0x20 f(ptr,int,ptr,ptr) / +0x28 f(int) / +0x30 f(ptr,ptr,ptr) / +0x38 f(ptr,ptr,int)
///   - `HWR_Init` 内部用 x0..x3（4 个指针）→ 模型名/词典名由**调用方**传入（不在 .so 里）
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeGpenTable(
    mut env: JNIEnv,
    _cls: JClass,
    path: JString,
) -> jstring {
    let p = jstr(&mut env, &path);
    let Ok(cpath) = std::ffi::CString::new(p) else {
        return out(&mut env, "bad path".into());
    };
    const RTLD_NOW: i32 = 2;
    const RTLD_GLOBAL: i32 = 0x00100;
    let h = unsafe { dlopen(cpath.as_ptr(), RTLD_NOW | RTLD_GLOBAL) };
    if h.is_null() {
        return out(&mut env, "dlopen failed".into());
    }
    let mut rep = String::new();
    let sym = std::ffi::CString::new("HWR_GetFunctions").unwrap();
    let f = unsafe { dlsym(h, sym.as_ptr()) };
    if f.is_null() {
        return out(&mut env, "HWR_GetFunctions 缺失".into());
    }
    // HWR_GetFunctions() -> const void* table
    let get_table: extern "C" fn() -> *const usize = unsafe { std::mem::transmute(f) };
    let table = get_table();
    if table.is_null() {
        return out(&mut env, "函数表为空（未初始化？）".into());
    }
    rep.push_str(&format!("函数表 @{:p}
", table));
    for i in 0..14 {
        let v = unsafe { *table.add(i) };
        rep.push_str(&format!("  [+0x{:02x}] = 0x{:x}
", i * 8, v));
    }
    out(&mut env, rep)
}


// ── 搜狗 gpen 试参（崩溃安全）────────────────────────────────────────────────
// HWR_Init 的参数含义未知，试错会 SIGSEGV。用 fork() 隔离：子进程里调一次（崩了只死子进程），
// 父进程 waitpid 拿"返回码 / 信号"。实测（华为 JDN2-AL00）：
//   (mgr,model,dict,0) → rc=0    Configure(0,0,0,false,false) → rc=-2    RealRecognize((-1,-1) 分隔的点列) → rc=0
extern "C" {
    fn fork() -> i32;
    fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    fn _exit(code: i32) -> !;
}

fn wifexited(status: i32) -> bool { (status & 0x7f) == 0 }
fn wexitstatus(status: i32) -> i32 { (status >> 8) & 0xff }
fn wifsignaled(status: i32) -> bool { ((status & 0x7f) + 1) as i8 >> 1 > 0 }
fn wtermsig(status: i32) -> i32 { status & 0x7f }

/// 崩溃安全地试 `HWR_Init` 的参数排列（每个组合 fork 一次）。
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeGpenTryInit(
    mut env: JNIEnv,
    _cls: JClass,
    path: JString,
    assets: JObject,
    model: JString,
    dict: JString,
) -> jstring {
    let p = jstr(&mut env, &path);
    let model_s = jstr(&mut env, &model);
    let dict_s = jstr(&mut env, &dict);
    let Ok(cpath) = std::ffi::CString::new(p) else {
        return out(&mut env, "bad path".into());
    };
    // AAssetManager_fromJava 在 libandroid.so；运行时 dlsym（不能链接期声明，否则 loadLibrary 失败）
    let mgr = unsafe {
        let la = dlopen(b"libandroid.so ".as_ptr(), 2);
        if la.is_null() { 0usize } else {
            let sym = std::ffi::CString::new("AAssetManager_fromJava").unwrap();
            let f = dlsym(la, sym.as_ptr());
            if f.is_null() { 0usize } else {
                let from_java: extern "C" fn(*mut std::ffi::c_void, jni::sys::jobject) -> *mut std::ffi::c_void =
                    std::mem::transmute(f);
                from_java(env.get_native_interface() as *mut std::ffi::c_void, assets.as_raw()) as usize
            }
        }
    };
    let h = unsafe { dlopen(cpath.as_ptr(), 2) };
    if h.is_null() {
        return out(&mut env, "dlopen failed".into());
    }
    let sym = std::ffi::CString::new("HWR_Init").unwrap();
    let f = unsafe { dlsym(h, sym.as_ptr()) };
    if f.is_null() {
        return out(&mut env, "HWR_Init 缺失".into());
    }
    let init_fn: extern "C" fn(usize, usize, usize, usize) -> i32 = unsafe { std::mem::transmute(f) };
    let cm = std::ffi::CString::new(model_s).unwrap();
    let cd = std::ffi::CString::new(dict_s).unwrap();
    let (pm, pd) = (cm.as_ptr() as usize, cd.as_ptr() as usize);
    let variants: [(usize, usize, usize, usize, &str); 4] = [
        (mgr, pm, pd, 0, "(mgr, model, dict, 0)"),
        (mgr, pm, pd, mgr, "(mgr, model, dict, mgr)"),
        (pm, mgr, pd, 0, "(model, mgr, dict, 0)"),
        (mgr, pd, pm, 0, "(mgr, dict, model, 0)"),
    ];
    let mut rep = format!("AAssetManager* = 0x{mgr:x}
");
    for (i, (a, b, c, d, label)) in variants.iter().enumerate() {
        let pid = unsafe { fork() };
        if pid == 0 {
            let rc = init_fn(*a, *b, *c, *d);
            unsafe { _exit((rc & 0xff) as i32) };
        } else if pid < 0 {
            rep.push_str(&format!("[{i}] fork 失败
"));
            continue;
        }
        let mut status: i32 = 0;
        unsafe { waitpid(pid, &mut status, 0) };
        let verdict = if wifexited(status) {
            format!("返回 rc={}", wexitstatus(status))
        } else if wifsignaled(status) {
            format!("崩溃 信号={}", wtermsig(status))
        } else {
            format!("未知 status=0x{status:x}")
        };
        rep.push_str(&format!("[{i}] {label} → {verdict}
"));
    }
    out(&mut env, rep)
}

/// **Android 端专用**：加载 mmap 内存索引（`<db>.midx`）。
///
/// 返回 1=已用上 mmap 索引（候选查询 µs 级），0=没找到/指纹不符（继续走 SQLite）。
/// `.midx` 由 PC 侧 `midxtool` 生成、随词库一起下发；不在设备上重建。
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeLoadMmap(
    mut env: JNIEnv,
    _cls: JClass,
    handle: jlong,
    db_path: JString,
) -> jint {
    if handle == 0 {
        return 0;
    }
    // 句柄由 nativeLoad 创建；索引装载是一次性写，之后只读。
    let e = unsafe { &mut *(handle as *mut ImeEngine) };
    let path = jstr(&mut env, &db_path);
    let (ok, _src) = e.load_mmap_index(&path);
    if ok {
        eprintln!("[prisir_ime] mmap index loaded");
        1
    } else {
        0
    }
}

/// 九宫格候选（数字串 2-9）。
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeQueryT9(
    mut env: JNIEnv, _cls: JClass, handle: jlong, digits: JString,
) -> jstring {
    let Some(engine) = engine_of(handle) else { return out(&mut env, "[]".into()) };
    let d = jstr(&mut env, &digits);
    let cands = engine.query_t9(&d);
    let arr: Vec<serde_json::Value> = cands
        .iter()
        .map(|(w, wt)| serde_json::json!({"word": w, "weight": wt}))
        .collect();
    out(&mut env, serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into()))
}

/// 九宫格（2026-11-19 重做）：数字串 → {"pinyins":[读音...],"words":[候选...]}。
///
/// 读音来自标准音节表的**精确展开**（"64" → mi/ni/oi），候选是这些读音在词库里的
/// 精确命中（干净、无前缀噪音）。数字串拼不出读音时 pinyins 为空、words 走 t9 前缀兜底。
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeT9Query(
    mut env: JNIEnv, _cls: JClass, handle: jlong, digits: JString,
) -> jstring {
    let Some(engine) = engine_of(handle) else {
        return out(&mut env, "{\"pinyins\":[],\"words\":[]}".into());
    };
    let d = jstr(&mut env, &digits);
    let r = engine.t9_query(&d);
    let words: Vec<serde_json::Value> = r
        .words
        .iter()
        .map(|(w, wt)| serde_json::json!({"word": w, "weight": wt}))
        .collect();
    let payload = serde_json::json!({ "pinyins": r.pinyins, "words": words });
    out(&mut env, payload.to_string())
}

/// 九宫格：数字串 → 读音（**不查库**，纯内存，微秒级）。
///
/// 2026-11-19：Android 主线程在按键后立刻调它，把首行从数字串变成拼音，
/// 不等候选查询返回 —— 九宫格首行空等几十毫秒，手感会明显比 26 键"卡"。
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeT9Readings(
    mut env: JNIEnv, _cls: JClass, handle: jlong, digits: JString,
) -> jstring {
    let Some(engine) = engine_of(handle) else {
        return out(&mut env, "[]".into());
    };
    let d = jstr(&mut env, &digits);
    let arr: Vec<serde_json::Value> = engine
        .t9_readings(&d)
        .into_iter()
        .map(serde_json::Value::String)
        .collect();
    out(&mut env, serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into()))
}

/// 九宫格：选定某个**读音**后的候选（该读音的精确词 + 以它开头的更长词）。
/// 不做模糊音、不做无关扩展 —— 用户点了「ni」就只该看到 ni 的词。
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeQueryReading(
    mut env: JNIEnv, _cls: JClass, handle: jlong, reading: JString,
) -> jstring {
    let Some(engine) = engine_of(handle) else { return out(&mut env, "[]".into()) };
    let p = jstr(&mut env, &reading);
    let cands = engine.query_reading_candidates(&p);
    let arr: Vec<serde_json::Value> = cands
        .iter()
        .map(|(w, wt)| serde_json::json!({"word": w, "weight": wt}))
        .collect();
    out(&mut env, serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into()))
}

/// **26 键**：由**候选词**反查读音（"你好" → "ni hao"），给候选栏第一行显示。
///
/// 与 `nativeResolvePinyin` 的区别：那个按**输入**反查，可能和候选排序打架
/// （实测 "nhxf"：jp 最高权重是「南湖蟹粉」，候选首选是「你好幸福」）。
/// 这个按**词**反查 → 第一行读音永远等于首候选的读音。
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativePinyinOf(
    mut env: JNIEnv, _cls: JClass, handle: jlong, word: JString,
) -> jstring {
    let Some(engine) = engine_of(handle) else {
        return std::ptr::null_mut();
    };
    let w = jstr(&mut env, &word);
    match engine.pinyin_of_word(&w) {
        Some(p) => out(&mut env, p),
        None => std::ptr::null_mut(),
    }
}

/// **26 键**：输入 → 读音解析（简拼的**可发现性**）。
///
/// 26 键打简拼（"nhjsg" → 「你好旧时光」）本来就能一次出多字词，但第一行只显示
/// 用户敲的字母，用户看不出被解读成了什么 → 这个能力等于不存在。
/// 这里返回词库里的读音（音节用空格分隔）：{"word":"你好旧时光","pinyin":"ni hao jiu shi guang"}。
/// 解析不出来返回 null（调用方继续显示原输入）。
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeResolvePinyin(
    mut env: JNIEnv, _cls: JClass, handle: jlong, input: JString,
) -> jstring {
    let Some(engine) = engine_of(handle) else {
        return std::ptr::null_mut();
    };
    let s = jstr(&mut env, &input);
    match engine.resolve_pinyin(&s) {
        Some((word, pinyin)) => {
            let payload = serde_json::json!({"word": word, "pinyin": pinyin});
            out(&mut env, payload.to_string())
        }
        None => std::ptr::null_mut(),
    }
}


#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeUserList(
    mut env: JNIEnv, _cls: JClass, handle: jlong,
) -> jstring {
    let Some(engine) = engine_of(handle) else { return out(&mut env, "[]".into()) };
    let rows = engine.db.list_user_words();
    let arr: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(k, v, w, seq)| serde_json::json!({"key": k, "value": v, "weight": w, "seq": seq}))
        .collect();
    out(&mut env, serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into()))
}

#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeUserAdd(
    mut env: JNIEnv, _cls: JClass, handle: jlong, pinyin: JString, word: JString,
) -> jboolean {
    let (Some(engine), p, w) = (engine_of(handle), jstr(&mut env, &pinyin), jstr(&mut env, &word)) else {
        return 0;
    };
    if p.is_empty() || w.is_empty() { return 0; }
    match engine.db.add_user_word(&p, &w, 1) {
        Ok(()) => 1,
        Err(_) => 0,
    }
}

#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeUserRemove(
    mut env: JNIEnv, _cls: JClass, handle: jlong, pinyin: JString, word: JString,
) -> jint {
    let (Some(engine), p, w) = (engine_of(handle), jstr(&mut env, &pinyin), jstr(&mut env, &word)) else {
        return 0;
    };
    engine.db.remove_user_word(&p, &w).unwrap_or(0) as jint
}

#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeDictSearchPage(
    mut env: JNIEnv, _cls: JClass, handle: jlong, term: JString, limit: jint, offset: jint,
) -> jstring {
    let Some(engine) = engine_of(handle) else { return out(&mut env, "[]".into()) };
    let t = jstr(&mut env, &term);
    let rows = engine.db.search_main_dict_page(&t, limit.max(1) as usize, offset.max(0) as usize);
    let arr: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(k, v, w, src)| serde_json::json!({"key": k, "value": v, "weight": w, "source": src}))
        .collect();
    out(&mut env, serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into()))
}

#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeDictCount(
    mut env: JNIEnv, _cls: JClass, handle: jlong, term: JString,
) -> jlong {
    let Some(engine) = engine_of(handle) else { return 0 };
    engine.db.search_main_dict_count(&jstr(&mut env, &term)) as jlong
}

#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeNameList(
    mut env: JNIEnv, _cls: JClass, handle: jlong, black: jboolean,
) -> jstring {
    let Some(engine) = engine_of(handle) else { return out(&mut env, "[]".into()) };
    let rows = if black != 0 { engine.db.get_blacklist() } else { engine.db.get_whitelist() };
    let arr: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(v, r)| serde_json::json!({"value": v, "reason": r}))
        .collect();
    out(&mut env, serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into()))
}

#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeNameAdd(
    mut env: JNIEnv, _cls: JClass, handle: jlong, black: jboolean, word: JString, reason: JString,
) -> jint {
    let (Some(engine), w, r) = (engine_of(handle), jstr(&mut env, &word), jstr(&mut env, &reason)) else {
        return 0;
    };
    let res = if black != 0 { engine.db.add_to_blacklist(&w, &r) } else { engine.db.add_to_whitelist(&w, &r) };
    res.unwrap_or(0) as jint
}

#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeNameRemove(
    mut env: JNIEnv, _cls: JClass, handle: jlong, black: jboolean, word: JString,
) -> jint {
    let (Some(engine), w) = (engine_of(handle), jstr(&mut env, &word)) else { return 0 };
    let res = if black != 0 { engine.db.remove_from_blacklist(&w) } else { engine.db.remove_from_whitelist(&w) };
    res.unwrap_or(0) as jint
}

#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeMainUpsert(
    mut env: JNIEnv, _cls: JClass, handle: jlong, pinyin: JString, word: JString, weight: jlong,
) -> jint {
    let (Some(engine), p, w) = (engine_of(handle), jstr(&mut env, &pinyin), jstr(&mut env, &word)) else {
        return 0;
    };
    match engine.main_upsert_word(&p, &w, weight) {
        Ok(true) => 1,
        _ => 0,
    }
}

#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeMainDelete(
    mut env: JNIEnv, _cls: JClass, handle: jlong, pinyin: JString, word: JString,
) -> jint {
    let (Some(engine), p, w) = (engine_of(handle), jstr(&mut env, &pinyin), jstr(&mut env, &word)) else {
        return 0;
    };
    engine.main_delete_word(&p, &w).unwrap_or(0) as jint
}

#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativePinyinFor(
    mut env: JNIEnv, _cls: JClass, handle: jlong, word: JString,
) -> jstring {
    let (Some(engine), w) = (engine_of(handle), jstr(&mut env, &word)) else {
        return std::ptr::null_mut();
    };
    match engine.db.pinyin_for_word(&w) {
        Some(k) => out(&mut env, k),
        None => std::ptr::null_mut(),
    }
}

/// 释放引擎句柄。
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeFree(
    _env: JNIEnv,
    _cls: JClass,
    handle: jlong,
) {
    if handle != 0 {
        unsafe { drop(Box::from_raw(handle as *mut ImeEngine)) };
    }
}

// ── P2 简繁输出模式（2026-11-19 Android 端接入）────────────────────────────
// 引擎自带 OpenCC 词表（s2t.rs：简→繁 42,460 条 / 繁→简 3,578 条），
// Android 端**不再自带一份词表**，直接用引擎这套（三端同源、词表已在 .so 内）。
//
// · nativeSetScript: 0=原样 1=简→繁 2=繁→简，返回设置后的模式
// · nativeScript:    读当前模式
// · nativeToScript:  按当前模式转换文本（上屏前统一走它）

/// 设置简繁输出模式，返回设置后的模式（0/1/2）。
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeSetScript(
    _env: JNIEnv,
    _cls: JClass,
    handle: jlong,
    mode: jint,
) -> jint {
    let Some(engine) = engine_of(handle) else {
        return 0;
    };
    engine.set_script(crate::engine::ScriptMode::from_i32(mode));
    engine.script().as_i32()
}

/// 读当前简繁输出模式（0/1/2）。
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeScript(
    _env: JNIEnv,
    _cls: JClass,
    handle: jlong,
) -> jint {
    let Some(engine) = engine_of(handle) else {
        return 0;
    };
    engine.script().as_i32()
}

/// 按当前简繁模式转换文本（原样模式下原样返回）。
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeToScript(
    mut env: JNIEnv,
    _cls: JClass,
    handle: jlong,
    text: JString,
) -> jstring {
    let Some(engine) = engine_of(handle) else {
        return out(&mut env, String::new());
    };
    let t = jstr(&mut env, &text);
    out(&mut env, engine.to_script(&t))
}
