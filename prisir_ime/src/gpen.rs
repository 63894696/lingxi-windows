//! 搜狗 gpen 手写引擎探针（仅内部对比用；闭源库，不随产品分发）。
//!
//! ABI 已在 `libsogouime.so` / `libgpen_handwriter.so` 上逆向确认（Obsidian《搜狗手写引擎gpen-ABI逆向阶段报告》）：
//!
//! ```text
//! HWR_GetFunctions() -> 表（槽位 = 导出，来自 gpen .rela.dyn）：
//!   +0x00 RealRecognize   +0x08 Configure     +0x10 Reset        +0x18 Context
//!   +0x20 GetAllRegResult +0x28 SetLangeVersion +0x30 Init        +0x38 Load_Param
//!   +0x40 Destroy          +0x48 Clear         +0x50 Version      +0x58 Beautify
//!
//! HWR_Init(p1,p2,p3,p4)              —— 4 指针（模型名/词典名由调用方给）
//! HWR_Configure(a,b,c,d,e)           —— b∈{1,3,4,5}（模式），c≤5（候选数），d/e 为 bool
//! HWR_RealRecognize(int* pts)        —— 坐标对，(−1,−1) 表示抬笔
//! HWR_GetAllRegResult(int* len,int view,int* a,int* b) -> char*
//! ```
//!
//! 纪律：**任何一次调用都必须 fork 隔离**（参数不对就是 SIGSEGV，不能拖垮输入法进程）。

use jni::objects::{JClass, JObject, JPrimitiveArray, JString};
use jni::sys::{jarray, jint, jintArray, jstring};
use jni::JNIEnv;

extern "C" {
    fn dlopen(filename: *const u8, flag: i32) -> *mut std::ffi::c_void;
    fn dlsym(handle: *mut std::ffi::c_void, symbol: *const u8) -> *mut std::ffi::c_void;
    fn dlerror() -> *mut u8;
    fn fork() -> i32;
    fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    fn _exit(code: i32) -> !;

    // libc 原语：用别名声明，避免与 std 同名符号冲突
    #[link_name = "pipe"]
    fn libc_pipe(fds: *mut i32) -> i32;
    #[link_name = "write"]
    fn libc_write(fd: i32, buf: *const u8, count: usize) -> isize;
    #[link_name = "read"]
    fn libc_read(fd: i32, buf: *mut u8, count: usize) -> isize;
    #[link_name = "close"]
    fn libc_close(fd: i32) -> i32;
}

/// libc 转发（带前缀，避免与 std 符号冲突）。
/// ⚠️ 必须真的**定义**它们：只声明不定义 → 链接出 undefined symbol →
///    app 启动时 dlopen libprisir_ime.so 失败 → 整个输入法进程崩。
#[no_mangle]
pub extern "C" fn c_pipe(fds: *mut i32) -> i32 {
    unsafe { libc_pipe(fds) }
}

#[no_mangle]
pub extern "C" fn c_write(fd: i32, buf: *const u8, count: usize) -> isize {
    unsafe { libc_write(fd, buf, count) }
}

#[no_mangle]
pub extern "C" fn c_read(fd: i32, buf: *mut u8, count: usize) -> isize {
    unsafe { libc_read(fd, buf, count) }
}

#[no_mangle]
pub extern "C" fn c_close(fd: i32) -> i32 {
    unsafe { libc_close(fd) }
}

const RTLD_NOW: i32 = 2;

type InitFn = unsafe extern "C" fn(usize, usize, usize, usize) -> i32;
type ConfFn = unsafe extern "C" fn(i32, i32, i32, i32, i32) -> i32;
type RecFn = unsafe extern "C" fn(*const i32) -> i32;
type GetAllFn = unsafe extern "C" fn(*mut i32, i32, *mut i32, *mut i32) -> *const u8;

// ── 无分配的输出（fork 出来的子进程里不能再碰分配器锁）────────────────────
unsafe fn wr(fd: i32, s: &[u8]) {
    let mut off = 0usize;
    while off < s.len() {
        let n = c_write(fd, s.as_ptr().add(off), s.len() - off);
        if n <= 0 {
            break;
        }
        off += n as usize;
    }
}

unsafe fn wr_str(fd: i32, s: &str) {
    wr(fd, s.as_bytes());
}

/// 写一个十进制整数（不分配内存）。
unsafe fn wr_i(fd: i32, v: i32) {
    let mut b = [0u8; 20];
    let mut n = 0usize;
    let neg = v < 0;
    let mut x = if neg { -(v as i64) } else { v as i64 };
    if x == 0 {
        b[0] = b'0';
        n = 1;
    } else {
        while x > 0 {
            b[n] = b'0' + (x % 10) as u8;
            x /= 10;
            n += 1;
        }
        if neg {
            b[n] = b'-';
            n += 1;
        }
    }
    let mut tmp = [0u8; 20];
    for i in 0..n {
        tmp[i] = b[n - 1 - i];
    }
    wr(fd, &tmp[..n]);
}

unsafe fn wr_hex(fd: i32, p: *const u8, len: usize) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut buf = [0u8; 1024];
    let mut o = 0usize;
    let mut i = 0usize;
    while i < len && o + 2 <= buf.len() {
        let v = *p.add(i);
        buf[o] = HEX[(v >> 4) as usize];
        buf[o + 1] = HEX[(v & 0xf) as usize];
        o += 2;
        i += 1;
    }
    wr(fd, &buf[..o]);
}

fn wifexited(status: i32) -> bool {
    (status & 0x7f) == 0
}
fn wexitstatus(status: i32) -> i32 {
    (status >> 8) & 0xff
}
fn wifsignaled(status: i32) -> bool {
    (((status & 0x7f) + 1) as i8) >> 1 > 0
}
fn wtermsig(status: i32) -> i32 {
    status & 0x7f
}

fn jstr(env: &mut JNIEnv, s: &JString) -> String {
    env.get_string(s).map(|v| v.into()).unwrap_or_default()
}

/// 读 int[]（笔迹/参数）到 Vec —— 必须在 JNI 线程里做，fork 后不能碰 JVM。
unsafe fn read_ints(env: &mut JNIEnv, arr: jintArray) -> Vec<i32> {
    if arr.is_null() {
        return Vec::new();
    }
    let ja = JPrimitiveArray::<jint>::from_raw(arr as jarray);
    let len = env.get_array_length(&ja).unwrap_or(0) as usize;
    if len == 0 {
        return Vec::new();
    }
    let mut v = vec![0i32; len];
    if env.get_int_array_region(&ja, 0, &mut v).is_ok() {
        v
    } else {
        Vec::new()
    }
}

fn out(env: &mut JNIEnv, s: String) -> jstring {
    env.new_string(s).map(|v| v.into_raw()).unwrap_or(std::ptr::null_mut())
}

/// 取 AAssetManager*（必须在 JNI 线程里取，fork 之后不能再碰 JVM）。
unsafe fn asset_manager(env: &mut JNIEnv, assets: &JObject) -> usize {
    let la = dlopen(b"libandroid.so\0".as_ptr(), RTLD_NOW);
    if la.is_null() {
        return 0;
    }
    let sym = std::ffi::CString::new("AAssetManager_fromJava").unwrap();
    let f = dlsym(la, sym.as_ptr());
    if f.is_null() {
        return 0;
    }
    let from_java: extern "C" fn(*mut std::ffi::c_void, jni::sys::jobject) -> *mut std::ffi::c_void =
        std::mem::transmute(f);
    from_java(env.get_native_interface() as *mut std::ffi::c_void, assets.as_raw()) as usize
}

struct Gpen {
    init: InitFn,
    conf: ConfFn,
    rec: RecFn,
    get_all: GetAllFn,
}

unsafe fn load_gpen(path: &str) -> Result<Gpen, String> {
    let cpath = std::ffi::CString::new(path).map_err(|_| "path 含非法字符".to_string())?;
    let h = dlopen(cpath.as_ptr(), RTLD_NOW);
    if h.is_null() {
        let e = {
            let p = dlerror();
            if p.is_null() {
                "未知错误".to_string()
            } else {
                std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        return Err(format!("dlopen 失败: {e}"));
    }
    let sym = |n: &str| -> *mut std::ffi::c_void {
        let c = std::ffi::CString::new(n).unwrap();
        dlsym(h, c.as_ptr())
    };
    let f_init = sym("HWR_Init");
    let f_conf = sym("HWR_Configure");
    let f_rec = sym("HWR_RealRecognize");
    let f_get = sym("HWR_GetAllRegResult");
    if f_init.is_null() || f_conf.is_null() || f_rec.is_null() || f_get.is_null() {
        return Err("HWR_* 符号缺失".into());
    }
    Ok(Gpen {
        init: std::mem::transmute(f_init),
        conf: std::mem::transmute(f_conf),
        rec: std::mem::transmute(f_rec),
        get_all: std::mem::transmute(f_get),
    })
}

/// 一次识别的完整流程（在子进程里跑；崩了也不影响父进程）。
///
/// `cfg` = (a,b,c,d,e)（Configure 的 5 个参数），`view` = GetAllRegResult 的第 2 参数。
unsafe fn run_once(
    fd: i32,
    g: &Gpen,
    strokes: *const i32,
    init_args: (usize, usize, usize, usize),
    cfg: (i32, i32, i32, i32, i32),
    view: i32,
) {
    let rc = (g.init)(init_args.0, init_args.1, init_args.2, init_args.3);
    wr_str(fd, "init_rc=");
    wr_i(fd, rc);
    wr_str(fd, "\n");
    if rc != 0 {
        _exit(0);
    }
    let rc = (g.conf)(cfg.0, cfg.1, cfg.2, cfg.3, cfg.4);
    wr_str(fd, "conf_rc=");
    wr_i(fd, rc);
    wr_str(fd, "\n");
    if rc != 0 {
        _exit(0);
    }
    let rc = (g.rec)(strokes);
    wr_str(fd, "rec_rc=");
    wr_i(fd, rc);
    wr_str(fd, "\n");
    if rc != 0 {
        _exit(0);
    }
    let mut len: i32 = 0;
    let mut i1: i32 = 0;
    let mut i2: i32 = 0;
    let p = (g.get_all)(&mut len as *mut i32, view, &mut i1 as *mut i32, &mut i2 as *mut i32);
    wr_str(fd, "view=");
    wr_i(fd, view);
    wr_str(fd, " len=");
    wr_i(fd, len);
    wr_str(fd, " i1=");
    wr_i(fd, i1);
    wr_str(fd, " i2=");
    wr_i(fd, i2);
    wr_str(fd, " data=");
    if p.is_null() {
        wr_str(fd, "NULL\n");
    } else {
        // 真实返回指针 = 数据首部 + 8（头部是两个 int32），长度另计
        let base = p.offset(-8);
        let n = if len > 0 { len as usize } else { 0 };
        wr_hex(fd, base, n);
        wr_str(fd, "\n");
    }
    _exit(0);
}

/// 扫参数：Init 变体 × Configure 组合 × GetAllRegResult 的 view。
/// 每一次危险调用都在独立 fork 里（崩溃只丢那一格）。
fn sweep(g: &Gpen, strokes: &[i32], mgr: usize, cmodel: *const u8, cdict: *const u8) -> String {
    let mut report = String::new();
    let inits: [(&str, (usize, usize, usize, usize)); 4] = [
        ("(mgr,model,dict,0)", (mgr, cmodel as usize, cdict as usize, 0)),
        ("(0,model,dict,0)", (0, cmodel as usize, cdict as usize, 0)),
        ("(mgr,model,0,dict)", (mgr, cmodel as usize, 0, cdict as usize)),
        ("(0,model,0,dict)", (0, cmodel as usize, 0, cdict as usize)),
    ];
    for (label, args) in inits.iter() {
        let mut fds = [0i32; 2];
        if unsafe { c_pipe(fds.as_mut_ptr()) } != 0 {
            report.push_str("pipe 失败\n");
            break;
        }
        let pid = unsafe { fork() };
        if pid == 0 {
            unsafe {
                c_close(fds[0]);
                let strokes_ptr = strokes.as_ptr();
                for b in [1i32, 3, 4, 5] {
                    for c in 0..=5i32 {
                        for d in 0..=1i32 {
                            for e in 0..=1i32 {
                                // 真正的识别（可能崩）——每次一个 fork
                                let p2 = fork();
                                if p2 == 0 {
                                    run_once(fds[1], g, strokes_ptr, *args, (0, b, c, d, e), 1);
                                } else if p2 < 0 {
                                    wr_str(fds[1], "fork2 失败\n");
                                } else {
                                    let mut st: i32 = 0;
                                    waitpid(p2, &mut st, 0);
                                    wr_str(fds[1], "--- cfg a=0 b=");
                                    wr_i(fds[1], b);
                                    wr_str(fds[1], " c=");
                                    wr_i(fds[1], c);
                                    wr_str(fds[1], " d=");
                                    wr_i(fds[1], d);
                                    wr_str(fds[1], " e=");
                                    wr_i(fds[1], e);
                                    wr_str(fds[1], " -> ");
                                    if wifexited(st) {
                                        wr_str(fds[1], "exit ");
                                        wr_i(fds[1], wexitstatus(st));
                                    } else if wifsignaled(st) {
                                        wr_str(fds[1], "信号 ");
                                        wr_i(fds[1], wtermsig(st));
                                    }
                                    wr_str(fds[1], "\n");
                                }
                            }
                        }
                    }
                }
                c_close(fds[1]);
                _exit(0);
            }
        } else if pid < 0 {
            report.push_str("fork 失败\n");
            unsafe { c_close(fds[0]); c_close(fds[1]); }
            continue;
        } else {
            unsafe { c_close(fds[1]); }
            let mut buf = vec![0u8; 1 << 16];
            let mut n = 0usize;
            loop {
                let r = unsafe { c_read(fds[0], buf.as_mut_ptr().add(n), buf.len() - n) };
                if r <= 0 {
                    break;
                }
                n += r as usize;
                if n >= buf.len() {
                    break;
                }
            }
            unsafe { c_close(fds[0]); }
            let mut st: i32 = 0;
            unsafe { waitpid(pid, &mut st, 0) };
            report.push_str(&format!("=== init {label} ===\n"));
            report.push_str(&String::from_utf8_lossy(&buf[..n]));
            if wifsignaled(st) {
                report.push_str(&format!("[init 进程崩溃，信号 {}]\n", wtermsig(st)));
            }
        }
    }
    report
}

/// **诊断入口**：按真实 ABI 扫描参数，找出「能喂笔迹 → 拿候选」的 Configure 组合。
///
/// 返回纯文本报告（Kotlin 状态页直接显示）。
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeGpenSweep(
    mut env: JNIEnv,
    _cls: JClass,
    path: JString,
    model: JString,
    dict: JString,
    strokes: jni::sys::jintArray,
    assets: JObject,
) -> jstring {
    let p = jstr(&mut env, &path);
    let model_s = jstr(&mut env, &model);
    let dict_s = jstr(&mut env, &dict);
    let g = match unsafe { load_gpen(&p) } {
        Ok(g) => g,
        Err(e) => return out(&mut env, e),
    };
    let cmodel = match std::ffi::CString::new(model_s.clone()) {
        Ok(c) => c,
        Err(_) => return out(&mut env, "模型名含非法字符".into()),
    };
    let cdict = match std::ffi::CString::new(dict_s.clone()) {
        Ok(c) => c,
        Err(_) => return out(&mut env, "词典名含非法字符".into()),
    };
    let mgr = unsafe { asset_manager(&mut env, &assets) };
    // 笔迹：int[] 坐标对，(−1,−1) 抬笔
    let pts: Vec<i32> = unsafe { read_ints(&mut env, strokes) };
    let report = sweep(&g, &pts, mgr, cmodel.as_ptr(), cdict.as_ptr());
    out(&mut env, report)
}

/// **生产入口**：固定参数跑一次识别，把结果字节以 hex 返回（Kotlin 侧按 GB18030 解码）。
///
/// 返回 JSON：`{"rc":..,"len":..,"i1":..,"i2":..,"hex":"..","err":".."}`
#[no_mangle]
pub extern "system" fn Java_com_lingxi_ime_Engine_nativeGpenRecognize(
    mut env: JNIEnv,
    _cls: JClass,
    path: JString,
    model: JString,
    dict: JString,
    strokes: jni::sys::jintArray,
    cfg: jni::sys::jintArray,
    assets: JObject,
) -> jstring {
    let p = jstr(&mut env, &path);
    let model_s = jstr(&mut env, &model);
    let dict_s = jstr(&mut env, &dict);
    let g = match unsafe { load_gpen(&p) } {
        Ok(g) => g,
        Err(e) => return out(&mut env, format!("{{\"err\":\"{}\"}}", e.replace('"', "'"))),
    };
    let cmodel = match std::ffi::CString::new(model_s) {
        Ok(c) => c,
        Err(_) => return out(&mut env, "{\"err\":\"模型名非法\"}".to_string()),
    };
    let cdict = match std::ffi::CString::new(dict_s) {
        Ok(c) => c,
        Err(_) => return out(&mut env, "{\"err\":\"词典名非法\"}".to_string()),
    };
    let pts: Vec<i32> = unsafe { read_ints(&mut env, strokes) };
    let mut cfgs = [0i32; 6];
    {
        let v: Vec<i32> = unsafe { read_ints(&mut env, cfg) };
        let n = v.len().min(6);
        cfgs[..n].copy_from_slice(&v[..n]);
    }
    let (a, b, c, d, e, view) = (cfgs[0], cfgs[1], cfgs[2], cfgs[3], cfgs[4], cfgs[5]);
    // Init 变体：带 AssetManager（模型以 **asset 名** 从 APK assets 读，gpen 只认这条路径）
    // / 不带；模型词典在 (p3,p4) 两种摆法。
    let mgr = unsafe { asset_manager(&mut env, &assets) };
    let inits: [(usize, usize, usize, usize); 4] = [
        (mgr, cmodel.as_ptr() as usize, cdict.as_ptr() as usize, 0),
        (mgr, cmodel.as_ptr() as usize, 0, cdict.as_ptr() as usize),
        (0, cmodel.as_ptr() as usize, cdict.as_ptr() as usize, 0),
        (0, cmodel.as_ptr() as usize, 0, cdict.as_ptr() as usize),
    ];
    let mut last = String::from("{\"err\":\"未执行\"}");
    for args in inits.iter() {
        let mut fds = [0i32; 2];
        if unsafe { c_pipe(fds.as_mut_ptr()) } != 0 {
            return out(&mut env, "{\"err\":\"pipe 失败\"}".to_string());
        }
        let pid = unsafe { fork() };
        if pid == 0 {
            unsafe {
                c_close(fds[0]);
                run_once(fds[1], &g, pts.as_ptr(), *args, (a, b, c, d, e), view);
            }
        }
        unsafe { c_close(fds[1]); }
        let mut buf = vec![0u8; 1 << 15];
        let mut n = 0usize;
        loop {
            let r = unsafe { c_read(fds[0], buf.as_mut_ptr().add(n), buf.len() - n) };
            if r <= 0 {
                break;
            }
            n += r as usize;
            if n >= buf.len() {
                break;
            }
        }
        unsafe { c_close(fds[0]); }
        let mut st: i32 = 0;
        unsafe { waitpid(pid, &mut st, 0) };
        let text = String::from_utf8_lossy(&buf[..n]).to_string();
        if wifsignaled(st) {
            last = format!("{{\"err\":\"崩溃 信号 {}\"}}", wtermsig(st));
            continue;
        }
        // 解析子进程输出：init_rc / conf_rc / rec_rc / view=.. len=.. i1=.. i2=.. data=<hex>
        let mut rc = -1i32;
        let mut len = 0i32;
        let mut i1 = 0i32;
        let mut i2 = 0i32;
        let mut hex = String::new();
        for line in text.lines() {
            let line = line.trim();
            if let Some(v) = line.strip_prefix("init_rc=") {
                rc = v.trim().parse().unwrap_or(-99);
            } else if let Some(v) = line.strip_prefix("conf_rc=") {
                rc = v.trim().parse().unwrap_or(-98);
            } else if let Some(v) = line.strip_prefix("rec_rc=") {
                rc = v.trim().parse().unwrap_or(-97);
            } else if line.starts_with("view=") {
                for part in line.split_whitespace() {
                    if let Some(v) = part.strip_prefix("len=") {
                        len = v.parse().unwrap_or(0);
                    } else if let Some(v) = part.strip_prefix("i1=") {
                        i1 = v.parse().unwrap_or(0);
                    } else if let Some(v) = part.strip_prefix("i2=") {
                        i2 = v.parse().unwrap_or(0);
                    } else if let Some(v) = part.strip_prefix("data=") {
                        hex = v.to_string();
                    }
                }
            }
        }
        if rc == 0 && !hex.is_empty() {
            last = format!(
                "{{\"rc\":0,\"len\":{len},\"i1\":{i1},\"i2\":{i2},\"hex\":\"{hex}\"}}"
            );
            break;
        } else if last.starts_with("{\"err\":\"未执行\"") {
            last = format!("{{\"rc\":{rc},\"len\":{len},\"hex\":\"{hex}\",\"err\":\"见日志\"}}");
        }
    }
    out(&mut env, last)
}
