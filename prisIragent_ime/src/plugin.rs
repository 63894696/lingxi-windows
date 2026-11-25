//! plugin.rs — 进程外插件框架(2026-09-05)
//!
//! **目标**:AI/皮肤/桌面宠物等扩展功能做成**进程外插件**(独立 exe),
//! 用户从网站下载解压到 `%LOCALAPPDATA%\Prisir\plugins\` 即用,
//! 不重装整个输入法;菜单/悬浮栏按钮按已安装插件**动态生成**。
//!
//! **为什么进程外(不 LoadLibrary dll 插件)**:
//!   - 崩溃隔离:插件崩只崩自己,不带垮 ctfmon/explorer/notepad(T25 已吃够进程内崩)。
//!   - 安全:dll 会注入每个宿主进程,杀软易报毒;独立 exe 行为干净。
//!   - 语言自由:AI 用 Python、皮肤用 WebView、宠物用任意框架,不需 Rust/ABI 兼容。
//!   - 动态增删:删插件目录即可,下次菜单触发检测「未安装」自动隐藏。
//!
//! **运行模型(对齐语音 trigger_voice,泛化而来)**:
//!   1. 输入法启动/菜单弹出前调 `available_plugins()` → 读 plugins.json,
//!      对每个 enabled 插件查 exe 是否存在,存在才进可用列表。
//!   2. 点按钮/菜单 → `trigger_plugin(id)`:
//!        - OpenEvent(event) 成功 → SetEvent(通知已运行插件 toggle,对齐语音激活模型);
//!        - 失败(ERROR_FILE_NOT_FOUND= 插件没在跑) → ShellExecute(exe) 拉起。
//!   3. AI 按钮 toggle(用户定):点 AI 按钮发 PrisirLingXi_AiToggle_Event,
//!      AI 窗激活后打字转发进 AI 窗;再点关闭。状态机在 AI 插件进程,输入法零侵入击键热路径。
//!
//! **plugins.json 位置**:`%LOCALAPPDATA%\Prisir\plugins.json`(用户可写,免管理员)。
//! 插件 exe 相对路径基于 `%LOCALAPPDATA%\Prisir\`。

use serde::Deserialize;
use std::path::PathBuf;
use windows::core::PCWSTR;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{OpenEventW, SetEvent, EVENT_MODIFY_STATE};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

fn logp(msg: &str) {
    crate::com_class_factory::log_dll_entry(&format!("[plugin] {}", msg));
}

/// 单个插件声明(plugins.json 数组元素)。
#[derive(Debug, Deserialize, Clone)]
pub struct PluginSpec {
    /// 稳定 id,如 "voice" / "ai" / "pet"。用于 trigger_plugin 定位。
    pub id: String,
    /// 显示名(菜单/提示),如 "语音听写" / "AI 助手"。
    pub name: String,
    /// 插件 exe 相对路径(基于 %LOCALAPPDATA%\Prisir\),如 "plugins\\voice\\lingxi_voice.exe"。
    pub exe: String,
    /// toggle 命名事件名。插件进程内应 CreateEvent 同名并监听。
    pub event: String,
    /// 悬浮栏按钮文字(1~2 字),如 "语" / "AI"。None= 不进悬浮栏只进菜单。
    #[serde(default)]
    pub button: Option<String>,
    /// 是否启用。false= 即使 exe 存在也不显示。
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct PluginsFile {
    #[serde(default)]
    plugins: Vec<PluginSpec>,
}

/// 插件根目录:`%LOCALAPPDATA%\Prisir`。无 LOCALAPPDATA 时退 C:\Temp。
pub fn plugin_root() -> PathBuf {
    std::env::var("LOCALAPPDATA")
        .map(|d| PathBuf::from(d).join("Prisir"))
        .unwrap_or_else(|_| PathBuf::from(r"C:\Temp\Prisir"))
}

fn plugins_json_path() -> PathBuf {
    plugin_root().join("plugins.json")
}

/// 诊断用:按「UTF-8 优先 + 系统 ANSI 兜底」读出 plugins.json 原文。
pub fn plugins_json_text() -> Option<String> {
    read_text_any_encoding(&plugins_json_path())
}

/// 读文本文件:UTF-8 优先,失败时按**系统 ANSI 代码页**解码。
///
/// 为什么必须容错(2026-11-25 找到的真根因):安装包用 NSIS `FileWrite` 写
/// `plugins.json`,NSIS 是按**系统 ANSI 代码页**落盘的。中文 Windows 上「语音听写」
/// 「语」「AI 助手」这些中文就写成了 GBK 字节,`fs::read_to_string`(只认 UTF-8)
/// 直接报 `stream did not contain valid UTF-8` → 整个插件列表解析失败返回空
/// → 悬浮栏没有「语」、菜单没有「语音听写」。
/// 用户看到的是「语音输入被弄丢了」,实际是**插件配置一条都没读进来**。
///
/// 现在:UTF-8 失败就走 `MultiByteToWideChar(CP_ACP)`,GBK/Big5/日文等本地代码页
/// 写的配置都能正确读;仍失败才返回 None(插件机制对打字是纯增量,不能因此崩)。
fn read_text_any_encoding(path: &PathBuf) -> Option<String> {
    let raw = std::fs::read(path).ok()?;
    // UTF-8 BOM
    let raw = raw.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).map(|s| s.to_vec()).unwrap_or(raw);
    if let Ok(s) = std::str::from_utf8(&raw) {
        return Some(s.to_string());
    }
    #[cfg(windows)]
    {
        use windows::Win32::Globalization::{MultiByteToWideChar, CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};
        let flags = MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0);
        let need = unsafe { MultiByteToWideChar(CP_ACP, flags, &raw, None) };
        if need <= 0 {
            return None;
        }
        let mut buf = vec![0u16; need as usize];
        let got = unsafe { MultiByteToWideChar(CP_ACP, flags, &raw, Some(&mut buf)) };
        if got <= 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..got as usize]))
    }
    #[cfg(not(windows))]
    {
        Some(String::from_utf8_lossy(&raw).into_owned())
    }
}

fn file_exists(path: &PathBuf) -> bool {
    // windows 0.58 的 GetFileAttributesW 返回 (),不能判 u32::MAX;用 std Path::exists 更直观。
    path.exists()
}

/// 读 plugins.json 并过滤出「已安装(enabled + exe 存在)」的可用插件。
/// 任何一步失败(json 缺失/解析错/exe 不在)都返回缩小后的列表,不报错 ——
/// 插件机制对核心输入是**纯增量**,绝不能让插件问题影响打字。
pub fn available_plugins() -> Vec<PluginSpec> {
    let json_path = plugins_json_path();
    let raw = match read_text_any_encoding(&json_path) {
        Some(s) => s,
        None => {
            // 无 plugins.json = 没装任何插件,正常态,不刷屏日志。
            return Vec::new();
        }
    };
    let parsed: PluginsFile = match serde_json::from_str(&raw) {
        Ok(p) => p,
        Err(e) => {
            // 容错:用户手写 Windows 路径常把 \ 当目录分隔符(非法 JSON 转义),
            // 自动把 \ 替换为 \\ 再试一次。仍失败则日志留因,返回空(绝不影响打字)。
            let fixed = raw.replace('\\', "\\\\");
            match serde_json::from_str(&fixed) {
                Ok(p) => {
                    logp("plugins.json auto-fixed backslashes (hint: use / or \\\\ in exe path)");
                    p
                }
                Err(e2) => {
                    logp(&format!("plugins.json parse FAIL {:?} -> {}", e2, json_path.display()));
                    return Vec::new();
                }
            }
        }
    };
    let root = plugin_root();
    let mut out = Vec::new();
    for spec in parsed.plugins {
        if !spec.enabled {
            continue;
        }
        let exe_abs = root.join(&spec.exe);
        if file_exists(&exe_abs) {
            out.push(spec);
        } else {
            logp(&format!(
                "plugin '{}' declared but exe missing: {}",
                spec.id,
                exe_abs.display()
            ));
        }
    }
    out
}

/// 按 id 查一个可用插件(供 trigger)。
fn find_plugin(id: &str) -> Option<PluginSpec> {
    available_plugins().into_iter().find(|p| p.id == id)
}

/// 语音听写插件的稳定 id。悬浮栏「语」按钮与菜单「语音听写」都是内置入口,
/// 但真正干活的是这个 id 对应的进程外插件(plugins.json 声明)。
pub const VOICE_PLUGIN_ID: &str = "voice";

/// 语音插件默认事件名(与 voice_input/lingxi_app.py 的 `_VOICE_EVENT` 一致)。
const VOICE_EVENT_DEFAULT: &str = "PrisirLingXi_VoiceToggle_Event";

/// 语音插件 exe 的候选路径(首个存在的即为所用)。
///
/// 为什么写死几条兜底:2026-09 把语音做成「纯插件」后,只要 plugins.json 缺失
/// (旧版安装包覆盖 / 用户手改坏 / LOCALAPPDATA 被清),语音入口就整条消失。
/// 现在内置按钮直接按这些默认位置找 exe,插件配置只是「可覆盖」的增强,
/// 而不是唯一真相。
fn voice_exe_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(pf) = std::env::var("ProgramFiles") {
        v.push(PathBuf::from(pf).join("PrisirIME").join("voice").join("lingxi_voice.exe"));
    }
    v.push(PathBuf::from(r"C:\Program Files\PrisirIME\voice\lingxi_voice.exe"));
    // 开发/免安装布局:LOCALAPPDATA\Prisir\plugins\voice\lingxi_voice.exe
    v.push(plugin_root().join("plugins").join("voice").join("lingxi_voice.exe"));
    v.push(plugin_root().join("voice").join("lingxi_voice.exe"));
    v
}

/// 只探测语音插件是否可解析(不拉起、不发事件)。`--plugins` 诊断用。
pub fn trigger_voice_probe() -> bool {
    if available_plugins().iter().any(|p| p.id == VOICE_PLUGIN_ID) {
        return true;
    }
    voice_exe_candidates().into_iter().any(|p| p.exists())
}

/// 触发语音听写(内置「语」按钮 / 菜单「语音听写」共用)。
///
/// 返回 true = 已发事件或已拉起 exe;false = 没找到可执行的语音插件。
/// 语义与 `trigger_plugin("voice")` 完全一致,只是 exe 解析多了兜底路径。
pub fn trigger_voice() -> bool {
    // 1) plugins.json 声明了 voice → 用它的 exe/event(用户可自定义路径)
    if let Some(spec) = available_plugins().into_iter().find(|p| p.id == VOICE_PLUGIN_ID) {
        signal_or_launch(&spec.event, &plugin_root().join(&spec.exe));
        return true;
    }
    // 2) 插件配置缺失/无效 → 按默认位置兜底(exe + 默认事件名)
    let Some(exe) = voice_exe_candidates().into_iter().find(|p| p.exists()) else {
        logp("trigger_voice: no voice plugin exe found (plugins.json + fallback paths all missing)");
        return false;
    };
    signal_or_launch(VOICE_EVENT_DEFAULT, &exe);
    true
}

/// 「已有实例就发命名事件 toggle,没有就 ShellExecute 拉起」—— 插件共用的一段。
fn signal_or_launch(event: &str, exe_abs: &PathBuf) {
    unsafe {
        let name: Vec<u16> = event.encode_utf16().chain(std::iter::once(0)).collect();
        match OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(name.as_ptr())) {
            Ok(h) if !h.is_invalid() => {
                let _ = SetEvent(h);
                let _ = CloseHandle(h);
                logp(&format!("signaled running plugin event '{}'", event));
            }
            _ => {
                let exe_w: Vec<u16> = exe_abs
                    .as_os_str()
                    .to_string_lossy()
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                let open: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
                // SW_SHOWNORMAL:插件可能是 .vbs/.bat 启动器(如 AI 壳 PrisirAI.vbs),
                // SW_HIDE 会把启动器拉起的窗口一并藏掉;正常显示让 GUI 插件自己决定显隐。
                let r = ShellExecuteW(
                    None,
                    PCWSTR(open.as_ptr()),
                    PCWSTR(exe_w.as_ptr()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    SW_SHOWNORMAL,
                );
                logp(&format!("launched plugin exe {} ret={:?}", exe_abs.display(), r));
            }
        }
    }
}

/// 通用插件 toggle:发命名事件;插件未运行则拉起 exe。
/// 与语音 trigger_voice 同构 —— 插件进程监听同名 event 做「按下激活/再按关闭」。
pub fn trigger_plugin(id: &str) {
    let spec = match find_plugin(id) {
        Some(s) => s,
        None => {
            logp(&format!("trigger '{}' but not available", id));
            return;
        }
    };
    signal_or_launch(&spec.event, &plugin_root().join(&spec.exe));
    logp(&format!("trigger plugin '{}' -> {} event={}", id, spec.exe, spec.event));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归:老 NSIS 安装包按系统 ANSI(中文 Windows = GBK/CP936)写 plugins.json,
    /// 若只认 UTF-8 会整份解析失败 → 插件列表为空 → 悬浮栏没有「语」、
    /// 菜单没有「语音听写」(用户 2026-11-25 报「语音输入被弄丢了」)。
    #[test]
    fn gbk_written_plugins_json_is_decoded() {
        let mut bytes = b"{\"plugins\":[{\"id\":\"voice\",\"name\":\"".to_vec();
        bytes.extend_from_slice(&[0xD3, 0xEF, 0xD2, 0xF4, 0xCC, 0xFD, 0xD0, 0xB4]); // 语音听写
        bytes.extend_from_slice(b"\",\"exe\":\"C:/x/lingxi_voice.exe\",\"event\":\"E\",\"button\":\"");
        bytes.extend_from_slice(&[0xD3, 0xEF]); // 语
        bytes.extend_from_slice(b"\",\"enabled\":true}]}");
        let path = std::env::temp_dir().join(format!(
            "prisir_plugins_gbk_test_{}.json",
            std::process::id()
        ));
        std::fs::write(&path, &bytes).unwrap();
        let s = read_text_any_encoding(&path).expect("GBK 字节必须能解码");
        assert!(s.contains("语音听写"), "解码结果: {s}");
        let parsed: PluginsFile = serde_json::from_str(&s).expect("解码后必须是合法 JSON");
        assert_eq!(parsed.plugins.len(), 1);
        assert_eq!(parsed.plugins[0].name, "语音听写");
        assert_eq!(parsed.plugins[0].button.as_deref(), Some("语"));
        let _ = std::fs::remove_file(&path);
    }

    /// UTF-8(带/不带 BOM)都能读,且 UTF-8 优先、不被 ANSI 解码破坏。
    #[test]
    fn utf8_plugins_json_is_decoded() {
        let body = "{\"plugins\":[{\"id\":\"ai\",\"name\":\"AI 助手\",\"exe\":\"C:/x/a.vbs\",\"event\":\"E\",\"enabled\":true}]}";
        let path = std::env::temp_dir().join(format!(
            "prisir_plugins_utf8_test_{}.json",
            std::process::id()
        ));
        std::fs::write(&path, body).unwrap();
        assert_eq!(read_text_any_encoding(&path).unwrap(), body);
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice(body.as_bytes());
        std::fs::write(&path, &bom).unwrap();
        assert_eq!(read_text_any_encoding(&path).unwrap(), body);
        let _ = std::fs::remove_file(&path);
    }
}
