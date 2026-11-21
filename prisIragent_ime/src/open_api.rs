//! open_api.rs — 本机开放 API（供 AI Agent 定制个性化输入法）
//!
//! **用途**:让本机 Agent / 脚本接入，按用户习惯协助查询状态、调整配置与词库相关能力。
//! 默认关闭；需要 Agent 接入时：`prisir_tsfsvc --open-api-config enable`。
//!
//! 设计边界:
//!   - 配置落 `%LOCALAPPDATA%\Prisir\IME\open_api.json`
//!   - 只绑 `127.0.0.1`（不提供 LAN bind）
//!   - HTTP 复用 `ipc::handle_request`（POST /rpc → JSON-RPC）
//!   - daemon 启动时若 `enabled=true` 自动起监听线程
//!   - `prisir_tsfsvc --open-api` 可单独前台冒烟（无视 enabled）
//!
//! 端点:
//!   GET  /           → 服务说明（含用途摘要）
//!   GET  /health     → {"ok":true,...}
//!   GET  /status     → 注册状态 + open_api 配置快照
//!   POST /rpc        → body = 一行 JSON-RPC（同 stdio ipc）
//!
//! 不做：鉴权 token、SSE、注入按键、LAN 绑定（后续增量）。

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

/// 默认端口（避开 lx-music 23330）。
pub const DEFAULT_PORT: u16 = 23331;
pub const DEFAULT_BIND: &str = "127.0.0.1";

static LISTENING: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);
static ACTIVE_PORT: Mutex<Option<u16>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_bind")]
    pub bind: String,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}
fn default_bind() -> String {
    DEFAULT_BIND.to_string()
}

impl Default for OpenApiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_PORT,
            bind: DEFAULT_BIND.to_string(),
        }
    }
}

/// `%LOCALAPPDATA%\Prisir\IME\open_api.json`；无 LOCALAPPDATA 时退 C:\Temp\Prisir\IME\。
pub fn config_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Temp"));
    base.join("Prisir").join("IME").join("open_api.json")
}

pub fn load_config() -> OpenApiConfig {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => OpenApiConfig::default(),
    }
}

pub fn save_config(cfg: &OpenApiConfig) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    // 强制本机绑定：骨架阶段不允许改成 0.0.0.0
    let mut cfg = cfg.clone();
    if cfg.bind != "127.0.0.1" && cfg.bind != "localhost" {
        cfg.bind = DEFAULT_BIND.to_string();
    }
    if cfg.port == 0 {
        cfg.port = DEFAULT_PORT;
    }
    let text = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

pub fn is_listening() -> bool {
    LISTENING.load(Ordering::SeqCst)
}

pub fn active_port() -> Option<u16> {
    *ACTIVE_PORT.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn request_stop() {
    STOP.store(true, Ordering::SeqCst);
}

fn set_listening(on: bool, port: Option<u16>) {
    LISTENING.store(on, Ordering::SeqCst);
    if let Ok(mut g) = ACTIVE_PORT.lock() {
        *g = port;
    }
}

/// daemon / CLI 入口：若配置 enabled（或 `force=true`）则后台起 HTTP。
/// 返回 Ok(true)=已启线程，Ok(false)=未启（配置关），Err=bind 失败。
pub fn maybe_start_from_config(force: bool) -> Result<bool, String> {
    let cfg = load_config();
    if !force && !cfg.enabled {
        println!(
            "[open_api] disabled (config: {})",
            config_path().display()
        );
        return Ok(false);
    }
    start_listener(&cfg.bind, cfg.port)
}

/// 前台阻塞跑 HTTP（`--open-api` 冒烟用）。Ctrl-C / STOP 退出。
pub fn run_foreground(bind: &str, port: u16) -> Result<(), String> {
    STOP.store(false, Ordering::SeqCst);
    let addr: SocketAddr = format!("{bind}:{port}")
        .parse()
        .map_err(|e| format!("bad addr {bind}:{port}: {e}"))?;
    let listener = TcpListener::bind(addr).map_err(|e| format!("bind {addr}: {e}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("set_nonblocking: {e}"))?;
    set_listening(true, Some(port));
    println!("[open_api] listening on http://{bind}:{port}/  (foreground)");
    println!("[open_api] endpoints: GET /health  GET /status  POST /rpc");
    while !STOP.load(Ordering::SeqCst) && !crate::daemon::is_shutdown() {
        match listener.accept() {
            Ok((stream, peer)) => {
                thread::spawn(move || {
                    if let Err(e) = serve_connection(stream) {
                        eprintln!("[open_api] {peer}: {e}");
                    }
                });
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                eprintln!("[open_api] accept: {e}");
                thread::sleep(Duration::from_millis(200));
            }
        }
    }
    set_listening(false, None);
    println!("[open_api] stopped");
    Ok(())
}

fn start_listener(bind: &str, port: u16) -> Result<bool, String> {
    if is_listening() {
        println!(
            "[open_api] already listening on port {}",
            active_port().unwrap_or(port)
        );
        return Ok(true);
    }
    STOP.store(false, Ordering::SeqCst);
    let addr: SocketAddr = format!("{bind}:{port}")
        .parse()
        .map_err(|e| format!("bad addr {bind}:{port}: {e}"))?;
    let listener = TcpListener::bind(addr).map_err(|e| format!("bind {addr}: {e}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("set_nonblocking: {e}"))?;
    set_listening(true, Some(port));
    println!("[open_api] listening on http://{bind}:{port}/");
    println!("[open_api] endpoints: GET /health  GET /status  POST /rpc");

    thread::spawn(move || {
        while !STOP.load(Ordering::SeqCst) && !crate::daemon::is_shutdown() {
            match listener.accept() {
                Ok((stream, peer)) => {
                    thread::spawn(move || {
                        if let Err(e) = serve_connection(stream) {
                            eprintln!("[open_api] {peer}: {e}");
                        }
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    eprintln!("[open_api] accept: {e}");
                    thread::sleep(Duration::from_millis(200));
                }
            }
        }
        set_listening(false, None);
        println!("[open_api] listener thread exit");
    });
    Ok(true)
}

fn serve_connection(mut stream: TcpStream) -> Result<(), String> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

    let mut buf = vec![0u8; 64 * 1024];
    let mut got = Vec::new();
    loop {
        let n = stream.read(&mut buf).map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            break;
        }
        got.extend_from_slice(&buf[..n]);
        if got.windows(4).any(|w| w == b"\r\n\r\n") {
            // 若有 Content-Length，继续读满 body
            if let Some(header_end) = find_header_end(&got) {
                let headers = std::str::from_utf8(&got[..header_end]).unwrap_or("");
                let content_len = parse_content_length(headers).unwrap_or(0);
                let body_have = got.len().saturating_sub(header_end + 4);
                if body_have >= content_len {
                    break;
                }
                // 继续读
                continue;
            }
            break;
        }
        if got.len() > 1024 * 1024 {
            return Err("request too large".into());
        }
    }

    let req = String::from_utf8_lossy(&got);
    let (method, path, body) = parse_http_request(&req)?;
    let (status, content_type, payload) = route(&method, &path, body);

    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n{payload}",
        payload.as_bytes().len()
    );
    stream
        .write_all(resp.as_bytes())
        .map_err(|e| format!("write: {e}"))?;
    Ok(())
}

fn find_header_end(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|w| w == b"\r\n\r\n")
}

fn parse_content_length(headers: &str) -> Option<usize> {
    for line in headers.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            return rest.trim().parse().ok();
        }
    }
    None
}

fn parse_http_request(raw: &str) -> Result<(String, String, String), String> {
    let header_end = raw.find("\r\n\r\n").unwrap_or(raw.len());
    let head = &raw[..header_end];
    let body = raw.get(header_end + 4..).unwrap_or("").to_string();
    let first = head.lines().next().ok_or("empty request")?;
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    // path 去掉 query
    let path = path.split('?').next().unwrap_or("/").to_string();
    Ok((method, path, body))
}

fn route(method: &str, path: &str, body: String) -> (u16, &'static str, String) {
    let method = method.to_ascii_uppercase();
    match (method.as_str(), path) {
        ("GET", "/") => (
            200,
            "application/json; charset=utf-8",
            json!({
                "service": "prisir-ime-open-api",
                "version": env!("CARGO_PKG_VERSION"),
                "purpose": "Local Open API for AI agents to help customize a personalized IME on this machine. Default off; enable with: prisir_tsfsvc --open-api-config enable",
                "purpose_zh": "供本机 AI Agent 定制个性化输入法；默认关闭；开启：prisir_tsfsvc --open-api-config enable",
                "endpoints": ["GET /health", "GET /status", "POST /rpc"],
                "bind": "127.0.0.1 only",
                "docs": "POST /rpc with JSON-RPC body (methods: version/status/register/unregister/enable/disable/query/open_api.get/open_api.set)",
            })
            .to_string(),
        ),
        ("GET", "/health") => (
            200,
            "application/json; charset=utf-8",
            json!({
                "ok": true,
                "listening": is_listening(),
                "port": active_port(),
                "pid": std::process::id(),
                "version": env!("CARGO_PKG_VERSION"),
            })
            .to_string(),
        ),
        ("GET", "/status") => {
            let cfg = load_config();
            let tip = crate::register::do_status().ok();
            (
                200,
                "application/json; charset=utf-8",
                json!({
                    "open_api": {
                        "enabled": cfg.enabled,
                        "port": cfg.port,
                        "bind": cfg.bind,
                        "listening": is_listening(),
                        "active_port": active_port(),
                        "config_path": config_path().display().to_string(),
                    },
                    "tip": tip.map(|s| json!({
                        "tip_key_exists": s.tip_key_exists,
                        "clsid_key_exists": s.clsid_key_exists,
                        "clsid": s.clsid,
                    })),
                    "pid": std::process::id(),
                    "version": env!("CARGO_PKG_VERSION"),
                })
                .to_string(),
            )
        }
        ("POST", "/rpc") => {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                return (
                    400,
                    "application/json; charset=utf-8",
                    json!({"error":{"code":-32700,"message":"empty body"}}).to_string(),
                );
            }
            let response = crate::ipc::handle_request(trimmed);
            (200, "application/json; charset=utf-8", response)
        }
        ("OPTIONS", _) => (
            204,
            "text/plain",
            String::new(),
        ),
        _ => (
            404,
            "application/json; charset=utf-8",
            json!({"error":"not found","path": path}).to_string(),
        ),
    }
}

/// 供 ipc JSON-RPC 扩展：`open_api.get` / `open_api.set`
pub fn rpc_get() -> serde_json::Value {
    let cfg = load_config();
    json!({
        "enabled": cfg.enabled,
        "port": cfg.port,
        "bind": cfg.bind,
        "listening": is_listening(),
        "active_port": active_port(),
        "config_path": config_path().display().to_string(),
    })
}

pub fn rpc_set(params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let mut cfg = load_config();
    if let Some(v) = params.get("enabled").and_then(|x| x.as_bool()) {
        cfg.enabled = v;
    }
    if let Some(v) = params.get("port").and_then(|x| x.as_u64()) {
        if v == 0 || v > 65535 {
            return Err("port out of range".into());
        }
        cfg.port = v as u16;
    }
    // bind 骨架固定 127.0.0.1
    cfg.bind = DEFAULT_BIND.to_string();
    save_config(&cfg)?;

    // 运行时启停：enabled=true 且未听 → 启；enabled=false 且在听 → 停（下次 daemon 才真正释放，骨架用 STOP）
    if cfg.enabled {
        if !is_listening() {
            let _ = start_listener(&cfg.bind, cfg.port)?;
        }
    } else if is_listening() {
        request_stop();
    }

    Ok(rpc_get())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_disabled() {
        let c = OpenApiConfig::default();
        assert!(!c.enabled);
        assert_eq!(c.port, DEFAULT_PORT);
        assert_eq!(c.bind, "127.0.0.1");
    }

    #[test]
    fn parse_get_health() {
        let raw = "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        let (m, p, b) = parse_http_request(raw).unwrap();
        assert_eq!(m, "GET");
        assert_eq!(p, "/health");
        assert!(b.is_empty());
    }
}
