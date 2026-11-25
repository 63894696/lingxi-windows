//! 输入法激活键 + 输入法切换 — 沿用 voice_input/lingxi_ime/backend 范式
//!
//! 设计目的:
//!   - 拼音 / 五笔 共享同一套 TSF 进程 + 同一套 ciku.db
//!   - 区分仅在「激活键」与「码表查询方式」(拼音=全拼/简拼,五笔=根码)
//!   - T7 阶段只把输入法切换能力落进 keystroke + main --method,
//!     真按激活键自动切换是 T8 沙盒 E2E 才接(本模块只暴露常量与解析,不接 TSF 事件流)
//!
//! 数值约定(严格沿用 voice_input/lingxi_ime/backend/hotkey_field.py + ime_config.py):
//!   - Shift:  VK_LSHIFT=0xA0  VK_RSHIFT=0xA1  → 通用 0x10
//!   - Ctrl:   VK_LCTRL =0xA2  VK_RCTRL =0xA3  → 通用 0x11
//!   - Alt:    VK_LALT  =0xA4  VK_RALT  =0xA5  → 通用 0x12
//!   - Win:    VK_LWIN  =0x5B  VK_RWIN  =0x5C  → 通用 0x5B (T7 暂未启用,留给 Win+某键 路径)
//!   - PrintScreen=0x2C Insert=0x2D Delete=0x2E Home=0x2F NumLock=0x90
//!
//! 默认激活键:
//!   - 拼音 = 右 Ctrl(0xA3)  —— 与 ime_config.PINYIN_TRIGGER_KEY 一致
//!   - 五笔 = 右 Shift(0xA1) —— 与 ime_config.WUBI_TRIGGER_KEY 一致
#![allow(dead_code)]

use std::str::FromStr;

/// 允许作为输入法激活键的 VK 集合(沿用 voice_input ALLOWED_VKS,扩展 PrintScreen/Insert/
/// Delete/Home/NumLock —— 这些是 lingxi_hotkeys 暂未列但本进程内便于调试加的)。
pub const ALLOWED_VKS: &[u16] = &[
    0x10, 0x11, 0x12, 0x14,              // Shift/Ctrl/Alt/CapsLock 通用(归并后)
    0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5,  // 左右 Shift/Ctrl/Alt/Win
    0x2C, 0x2D, 0x2E, 0x2F,              // PrintScreen/Insert/Delete/Home
    0x90,                                  // NumLock
];

/// 拼音激活键 = 右 Ctrl(沿用 ime_config.PINYIN_TRIGGER_KEY = 0xA3)
pub const PINYIN_TRIGGER_KEY: u16 = 0xA3;

/// 五笔激活键 = 右 Shift(沿用 ime_config.WUBI_TRIGGER_KEY = 0xA1)
pub const WUBI_TRIGGER_KEY: u16 = 0xA1;

/// 当前输入法枚举。T7 阶段仅两态,语音激活在 T7 后做增量(不在 T7 范围)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMethod {
    Pinyin,
    Wubi,
}

impl InputMethod {
    /// 序列化用字符串(给 --status / ipc 输出)。固定小写英文,与 parse_method 入参对齐。
    pub fn as_str(self) -> &'static str {
        match self {
            InputMethod::Pinyin => "pinyin",
            InputMethod::Wubi   => "wubi",
        }
    }
}

/// 输入法名串 → 枚举。接受拼音/中文/简写(大小写不敏感)。
/// 失败时返 `Err(msg)`,给 main.rs `--method` 用作 stderr + 退出码 1。
pub fn parse_method(s: &str) -> Result<InputMethod, String> {
    let m = InputMethod::from_str(s)?;
    Ok(m)
}

impl FromStr for InputMethod {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pinyin" | "拼音" | "py" => Ok(InputMethod::Pinyin),
            "wubi" | "五笔" | "wb"   => Ok(InputMethod::Wubi),
            other => Err(format!("unknown method: {other}")),
        }
    }
}

/// VK 归一化(还原被归并的左右修饰键),再判断是否在 ALLOWED_VKS 内。
/// - 0xA0/0xA1 → 0x10 (Shift,通用)
/// - 0xA2/0xA3 → 0x11 (Ctrl,通用)
/// - 0xA4/0xA5 → 0x12 (Alt,通用)
/// - 0x5B/0x5C → 0x5B (Win,通用)
/// - 其他:原样返回
/// 在 ALLOWED_VKS 内才返 `Some(norm)`,否则 `None`。
pub fn normalize_vk(raw: u16) -> Option<u16> {
    let normalized = match raw {
        0xA0 | 0xA1 => 0x10,
        0xA2 | 0xA3 => 0x11,
        0xA4 | 0xA5 => 0x12,
        0x5B | 0x5C => 0x5B,
        other => other,
    };
    if ALLOWED_VKS.contains(&normalized) {
        Some(normalized)
    } else {
        None
    }
}

// ============================================================
// 快捷造词热键(2026-11-25 改为用户可配)
//
// **为什么必须可配**:默认 `Ctrl+Alt+E` 在测试机上被别的软件占了 —— 按下先被对方吃掉,
// 输入法根本收不到,功能等于不存在。可配之后用户能换成任意空出来的组合;真想彻底
// 不要热键,就 `--quick-add-hotkey off`,改从托盘菜单「造词」进(同一个窗口、同一张表)。
//
// 配置落 `%LOCALAPPDATA%\Prisir\IME\quick_add_hotkey.json`,不用管理员权限。
// 读配置带 1 秒 TTL 缓存:改完配置不用重启应用,下一个键就生效,但也不会每次按键都读盘。
// ============================================================

/// 快捷造词的按键组合。`enabled=false` 表示只用菜单入口、不注册任何热键。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickAddHotkey {
    pub enabled: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// 虚拟键码(A-Z = 0x41-0x5A,F1-F24 = 0x70-0x87,数字 0x30-0x39)
    pub vk: u16,
}

impl Default for QuickAddHotkey {
    fn default() -> Self {
        // 历史默认值,未配过的老用户行为不变。
        Self { enabled: true, ctrl: true, alt: true, shift: false, win: false, vk: 0x45 }
    }
}

impl QuickAddHotkey {
    /// 解析 `Ctrl+Alt+E` / `Alt+Q` / `off` / `disabled` / 空串(=关闭)。
    ///
    /// 拒绝「没有 Ctrl/Alt/Win 的组合」:单 Shift+字母、裸字母会被输入法自己吃掉,
    /// 直接毁掉打字 —— 这种配置必须当场报错,不能静默生效。
    pub fn parse(s: &str) -> Result<Self, String> {
        let t = s.trim();
        if t.is_empty() {
            return Ok(Self { enabled: false, ..Self::default() });
        }
        if matches!(
            t.to_ascii_lowercase().as_str(),
            "off" | "none" | "disable" | "disabled" | "关闭"
        ) {
            return Ok(Self { enabled: false, ..Self::default() });
        }
        let (mut ctrl, mut alt, mut shift, mut win) = (false, false, false, false);
        let mut vk: Option<u16> = None;
        for raw in t.split('+') {
            let part = raw.trim();
            if part.is_empty() {
                continue;
            }
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "ctl" => ctrl = true,
                "alt" | "menu" => alt = true,
                "shift" => shift = true,
                "win" | "meta" | "super" => win = true,
                k => {
                    if vk.is_some() {
                        return Err(format!("组合里只能有一个主键:{t}"));
                    }
                    vk = Some(parse_key(k)?);
                }
            }
        }
        let Some(vk) = vk else {
            return Err(format!("缺少主键(如 E / Q / F9):{t}"));
        };
        if !ctrl && !alt && !win {
            return Err(format!(
                "至少要带 Ctrl / Alt / Win 之一:{t} —— 裸键或纯 Shift 组合会被输入法吃掉,不能当热键"
            ));
        }
        Ok(Self { enabled: true, ctrl, alt, shift, win, vk })
    }

    /// 人类可读形式,直接给菜单/状态栏/CLI 显示。
    pub fn describe(&self) -> String {
        if !self.enabled {
            return "已关闭(从菜单「造词」进入)".to_string();
        }
        let mut parts: Vec<String> = Vec::new();
        if self.ctrl { parts.push("Ctrl".into()); }
        if self.alt { parts.push("Alt".into()); }
        if self.shift { parts.push("Shift".into()); }
        if self.win { parts.push("Win".into()); }
        parts.push(vk_name(self.vk));
        parts.join("+")
    }

    /// 按键是否命中本热键。修饰键按**精确**匹配:配了 Ctrl+Alt+E 时 Ctrl+E 不算,
    /// 配了 Ctrl+Shift+K 时 Ctrl+K 也不算 —— 宁可少触发,不要抢用户的其它快捷键。
    pub fn matches(&self, vk: u16, ctrl: bool, alt: bool, shift: bool, win: bool) -> bool {
        self.enabled
            && vk == self.vk
            && ctrl == self.ctrl
            && alt == self.alt
            && shift == self.shift
            && win == self.win
    }
}

fn parse_key(k: &str) -> Result<u16, String> {
    let bytes = k.as_bytes();
    if bytes.len() == 1 {
        let b = bytes[0].to_ascii_uppercase();
        if b.is_ascii_uppercase() || b.is_ascii_digit() {
            return Ok(b as u16);
        }
        return match b {
            b',' => Ok(0xBC),
            b'.' => Ok(0xBE),
            b'/' => Ok(0xBF),
            b';' => Ok(0xBA),
            b'\'' => Ok(0xDE),
            b'[' => Ok(0xDB),
            b']' => Ok(0xDD),
            b'\\' => Ok(0xDC),
            b'-' => Ok(0xBD),
            b'=' => Ok(0xBB),
            b'`' => Ok(0xC0),
            _ => Err(format!("不支持的键:{k}")),
        };
    }
    // F1-F24
    let lk = k.to_ascii_lowercase();
    if let Some(num) = lk.strip_prefix('f') {
        if let Ok(n) = num.parse::<u16>() {
            if (1..=24).contains(&n) {
                return Ok(0x70 + n - 1);
            }
        }
    }
    Err(format!("不认识的键名:{k}(支持 A-Z、0-9、F1-F24、常用符号)"))
}

fn vk_name(vk: u16) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => (vk as u8 as char).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        0xBC => ",".into(),
        0xBE => ".".into(),
        0xBF => "/".into(),
        0xBA => ";".into(),
        0xDE => "'".into(),
        0xDB => "[".into(),
        0xDD => "]".into(),
        0xDC => "\\".into(),
        0xBD => "-".into(),
        0xBB => "=".into(),
        0xC0 => "`".into(),
        other => format!("VK{other:02X}"),
    }
}

/// `%LOCALAPPDATA%\Prisir\IME\quick_add_hotkey.json`(与 open_api.json 同目录)。
pub fn quick_add_config_path() -> std::path::PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Temp"));
    base.join("Prisir").join("IME").join("quick_add_hotkey.json")
}

fn read_quick_add_file() -> Option<QuickAddHotkey> {
    let text = std::fs::read_to_string(quick_add_config_path()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
    if !enabled {
        return Some(QuickAddHotkey { enabled: false, ..QuickAddHotkey::default() });
    }
    let vk = v.get("vk").and_then(|x| x.as_i64()).unwrap_or(0x45) as u16;
    let combo = QuickAddHotkey {
        enabled: true,
        ctrl: v.get("ctrl").and_then(|x| x.as_bool()).unwrap_or(true),
        alt: v.get("alt").and_then(|x| x.as_bool()).unwrap_or(true),
        shift: v.get("shift").and_then(|x| x.as_bool()).unwrap_or(false),
        win: v.get("win").and_then(|x| x.as_bool()).unwrap_or(false),
        vk,
    };
    // 手改了 JSON 把修饰键全关掉 → 退回默认,不让输入法吃掉裸字母。
    if !combo.ctrl && !combo.alt && !combo.win {
        return Some(QuickAddHotkey::default());
    }
    Some(combo)
}

/// 读当前配置(1 秒 TTL 缓存)。配置坏了/不存在 → 默认 Ctrl+Alt+E。
pub fn load_quick_add() -> QuickAddHotkey {
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    static CACHE: OnceLock<Mutex<(Instant, QuickAddHotkey)>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| {
        Mutex::new((Instant::now() - Duration::from_secs(5), QuickAddHotkey::default()))
    });
    let mut g = cache.lock().unwrap_or_else(|e| e.into_inner());
    if g.0.elapsed() < Duration::from_secs(1) {
        return g.1.clone();
    }
    let cfg = read_quick_add_file().unwrap_or_default();
    *g = (Instant::now(), cfg.clone());
    cfg
}

pub fn save_quick_add(cfg: &QuickAddHotkey) -> Result<(), String> {
    let path = quick_add_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let text = serde_json::json!({
        "enabled": cfg.enabled,
        "ctrl": cfg.ctrl,
        "alt": cfg.alt,
        "shift": cfg.shift,
        "win": cfg.win,
        "vk": cfg.vk,
        "human": cfg.describe(),
        "_note": "快捷造词热键。改完即生效(无需重启);enabled=false 表示只用托盘菜单「造词」。",
    });
    std::fs::write(&path, serde_json::to_string_pretty(&text).map_err(|e| e.to_string())?)
        .map_err(|e| format!("write {}: {e}", path.display()))
}

// ============================================================
// 单元测试 — 不依赖 ffi / 不依赖 TSF,只测纯函数
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_vk_merges_modifier_pairs() {
        assert_eq!(normalize_vk(0xA0), Some(0x10), "VK_LSHIFT -> 通用 Shift");
        assert_eq!(normalize_vk(0xA1), Some(0x10), "VK_RSHIFT -> 通用 Shift");
        assert_eq!(normalize_vk(0xA2), Some(0x11), "VK_LCTRL -> 通用 Ctrl");
        assert_eq!(normalize_vk(0xA3), Some(0x11), "VK_RCTRL -> 通用 Ctrl");
        assert_eq!(normalize_vk(0xA4), Some(0x12), "VK_LALT -> 通用 Alt");
        assert_eq!(normalize_vk(0xA5), Some(0x12), "VK_RALT -> 通用 Alt");
    }

    #[test]
    fn normalize_vk_rejects_non_allowed() {
        assert_eq!(normalize_vk(0x41), None, "VK_A 不允许");
        assert_eq!(normalize_vk(0x20), None, "VK_SPACE 不允许");
        assert_eq!(normalize_vk(0x1B), None, "VK_ESC 不允许");
        assert_eq!(normalize_vk(0x00), None, "VK_NUL 不允许");
    }

    #[test]
    fn trigger_keys_match_voice_input() {
        assert_eq!(PINYIN_TRIGGER_KEY, 0xA3, "拼音激活键必须=右Ctrl(沿用 voice_input)");
        assert_eq!(WUBI_TRIGGER_KEY,   0xA1, "五笔激活键必须=右Shift(沿用 voice_input)");
    }

    #[test]
    fn parse_method_roundtrip() {
        assert_eq!(parse_method("pinyin").unwrap(), InputMethod::Pinyin);
        assert_eq!(parse_method("Pinyin").unwrap(), InputMethod::Pinyin);
        assert_eq!(parse_method("拼音").unwrap(),    InputMethod::Pinyin);
        assert_eq!(parse_method("py").unwrap(),      InputMethod::Pinyin);
        assert_eq!(parse_method("wubi").unwrap(),    InputMethod::Wubi);
        assert_eq!(parse_method("WUBI").unwrap(),    InputMethod::Wubi);
        assert_eq!(parse_method("五笔").unwrap(),    InputMethod::Wubi);
        assert_eq!(parse_method("wb").unwrap(),      InputMethod::Wubi);
        assert!(parse_method("foo").is_err(),       "未知 method 必须 Err");
        assert!(parse_method("").is_err(),          "空串必须 Err");
    }

    #[test]
    fn method_as_str_canonical() {
        assert_eq!(InputMethod::Pinyin.as_str(), "pinyin");
        assert_eq!(InputMethod::Wubi.as_str(),   "wubi");
    }

    // ── 快捷造词热键解析(2026-11-25)────────────────────────────────────────

    #[test]
    fn quick_add_parse_default_and_off() {
        let d = QuickAddHotkey::parse("Ctrl+Alt+E").unwrap();
        assert_eq!(d, QuickAddHotkey::default(), "Ctrl+Alt+E 必须等于历史默认值");
        assert_eq!(d.describe(), "Ctrl+Alt+E");
        assert!(!QuickAddHotkey::parse("off").unwrap().enabled);
        assert!(!QuickAddHotkey::parse("").unwrap().enabled, "空串=关闭");
        assert!(!QuickAddHotkey::parse("关闭").unwrap().enabled);
    }

    #[test]
    fn quick_add_parse_variants() {
        let c = QuickAddHotkey::parse("alt+shift+q").unwrap();
        assert_eq!((c.alt, c.shift, c.ctrl, c.vk), (true, true, false, 0x51));
        assert_eq!(c.describe(), "Alt+Shift+Q", "顺序固定,不随输入顺序漂移");
        let f = QuickAddHotkey::parse("Ctrl+F9").unwrap();
        assert_eq!((f.vk, f.describe()), (0x78, "Ctrl+F9".to_string()));
        let s = QuickAddHotkey::parse("Ctrl+,").unwrap();
        assert_eq!(s.vk, 0xBC);
    }

    #[test]
    fn quick_add_rejects_dangerous_combos() {
        // 裸字母 / 纯 Shift+字母会被输入法自己吃掉,必须拒绝而不是静默生效。
        assert!(QuickAddHotkey::parse("E").is_err());
        assert!(QuickAddHotkey::parse("Shift+E").is_err());
        assert!(QuickAddHotkey::parse("Ctrl+Alt").is_err());
        assert!(QuickAddHotkey::parse("Ctrl+E+Q").is_err());
        assert!(QuickAddHotkey::parse("Ctrl+Nope").is_err());
    }

    #[test]
    fn quick_add_matches_requires_exact_modifiers() {
        let c = QuickAddHotkey::parse("Ctrl+Alt+E").unwrap();
        assert!(c.matches(0x45, true, true, false, false));
        assert!(!c.matches(0x45, true, false, false, false), "Ctrl+E 不该触发");
        assert!(!c.matches(0x45, true, true, true, false), "多按 Shift 不该触发");
        assert!(!c.matches(0x51, true, true, false, false));
        let off = QuickAddHotkey { enabled: false, ..c };
        assert!(!off.matches(0x45, true, true, false, false), "关掉后永不触发");
    }
}