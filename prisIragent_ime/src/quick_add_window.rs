//! 快捷造词窗口(2026-11-24)。
//!
//! **出处**:这一功能的形态直接借鉴影子输入法 `Lib/srf_func.ahk::srf_create()`
//! (上游 https://gitee.com/orz707/Yzime ,GitHub 镜像 sudoec/Yzime,热键 `!CapsLock`)。
//! 借过来的是**交互骨架**,不是它的实现方式:
//!
//! | 影子输入法 | 这里 |
//! |---|---|
//! | 全局 AHK 热键 `Alt+CapsLock` | `Ctrl+Alt+E`,在我们的 TSF `OnKeyDown` 里开一个窄例外 |
//! | 拼音缓冲在 AHK 进程内,直接当编码 | 当前 composition buffer 当编码提示;更常用的是**中文自动拆简拼** |
//! | 位置 → 换算 weight 写主库 | 位置 → `pos` 列,由引擎在候选组装时插位(主库只读也生效) |
//! | 词条取剪贴板 → 上次上屏 | 词条取剪贴板(选中内容 → Ctrl+C 即可) |
//! | 独立进程,窗口不阻塞宿主 | 同进程窗口,所以**不跑模态消息循环**,只用子类化接 Enter/Esc |
//!
//! **为什么位置不用权重**:现网主库在 `C:\Program Files\PrisirIME\models\ciku.db` 是只读的,
//! `main_upsert_word` 会退化成写 user_word 并忽略传入权重 → 按权重算的「第 N 位」在真实
//! 安装上完全不生效。详见 `prisir_ime/src/db.rs::add_manual_word` 的注释。
//!
//! **窗口形态**:固定大小、无最大化;三行输入 + 一个「继续造词」勾选 + 保存/取消。
//! Enter 保存、Esc 退出,与影子的键盘流一致(不用碰鼠标)。

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{BST_CHECKED, BST_UNCHECKED, EM_SETSEL};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;

const WIN_CLASS: &str = "PrisirQuickAdd";
/// 标题栏顺便当提示行(影子的做法,省一行客户区高度)。
const WIN_TITLE: &str = "灵犀造词 — Enter 确定 · Esc 退出";

const IDC_QA_WORD: u16 = 3101;
const IDC_QA_CODE: u16 = 3102;
const IDC_QA_POS: u16 = 3103;
const IDC_QA_MORE: u16 = 3104;
const IDC_QA_SAVE: u16 = 3105;
const IDC_QA_CANCEL: u16 = 3106;
const IDC_QA_STATUS: u16 = 3107;
const IDC_QA_L_WORD: u16 = 3111;
const IDC_QA_L_CODE: u16 = 3112;
const IDC_QA_L_POS: u16 = 3113;
const IDC_QA_L_HINT: u16 = 3114;
const IDC_QA_L_HINT2: u16 = 3115;
const IDC_QA_L_KEYS: u16 = 3116;

// 苹果风配色(与 userdict_window / candidate_window 统一,BGR 0x00BBGGRR)。
const COL_BG: u32 = 0x00FAF9F7;
const COL_TEXT: u32 = 0x005A544D;
const COL_HINT: u32 = 0x008C857C;
const COL_ACCENT: u32 = 0x00A9750A;

const CLIENT_W: i32 = 520;
/// 客户区高度(2026-11-25 加高):原先 200 且状态行只有一行高,
/// 「先复制词条(选中 → Ctrl+C)…」这种长提示被裁掉半行 —— 用户实测反馈。
/// 现在状态行独占一行、宽 488、高 2 行,窗口足量留白。
///
/// 实测(GDI MeasureString,12pt Microsoft YaHei UI,宽 488):
///   最长的「…再从托盘菜单「造词」打开;…」= 562.9px → 2 行,每行 22.3px → 44.6px。
///   故 status_h 必须 ≥ 45(原先 42 会切掉第二行底部)。这里给到 48,留 3px 余量。
const CLIENT_H: i32 = 228;
const MARGIN: i32 = 16;
const ROW_H: i32 = 28;
const LABEL_W: i32 = 44;
const GAP: i32 = 10;

const EN_CHANGE: u32 = 0x0300;
const VK_RETURN_U: usize = 0x0D;
const VK_ESCAPE_U: usize = 0x1B;

struct QuickState {
    hwnd: Option<HWND>,
    /// 编码是否仍处于「自动推导」状态。用户一旦手改过编码就置 false,
    /// 之后改词条不再覆盖他手填的编码。
    code_auto: bool,
    /// 我们自己写控件文本时置 true,避免 EN_CHANGE 回灌成「用户手改」。
    setting: bool,
}

impl QuickState {
    fn new() -> Self {
        Self { hwnd: None, code_auto: true, setting: false }
    }
}

static STATE: OnceLock<Arc<Mutex<RefCell<QuickState>>>> = OnceLock::new();
unsafe impl Send for QuickState {}
unsafe impl Sync for QuickState {}

/// 独立进程模式(`prisir_tsfsvc --quick-add`):本进程没有 TSF 宿主的消息循环,
/// 要自己跑 GetMessage/DispatchMessage;关闭窗口时用 PostQuitMessage 退出。
static STANDALONE: AtomicBool = AtomicBool::new(false);

fn global_state() -> Arc<Mutex<RefCell<QuickState>>> {
    STATE.get_or_init(|| Arc::new(Mutex::new(RefCell::new(QuickState::new())))).clone()
}

fn wcs(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn cached_font() -> HFONT {
    static F: OnceLock<usize> = OnceLock::new();
    let raw = *F.get_or_init(|| unsafe {
        let face = wcs("Microsoft YaHei UI");
        CreateFontW(
            -16, 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0,
            DEFAULT_CHARSET.0 as u32, OUT_DEFAULT_PRECIS.0 as u32, CLIP_DEFAULT_PRECIS.0 as u32,
            CLEARTYPE_QUALITY.0 as u32, DEFAULT_PITCH.0 as u32, PCWSTR(face.as_ptr()),
        ).0 as usize
    });
    HFONT(raw as *mut _)
}

fn bg_brush() -> HBRUSH {
    static B: OnceLock<usize> = OnceLock::new();
    let raw = *B.get_or_init(|| unsafe { CreateSolidBrush(COLORREF(COL_BG)).0 as usize });
    HBRUSH(raw as *mut _)
}

fn edit_brush() -> HBRUSH {
    static B: OnceLock<usize> = OnceLock::new();
    let raw = *B.get_or_init(|| unsafe { CreateSolidBrush(COLORREF(0x00FFFFFF)).0 as usize });
    HBRUSH(raw as *mut _)
}

// ── 公开入口 ────────────────────────────────────────────────────────────────

/// 打开造词窗口。`hint` = 当前 composition buffer(纯字母),可为空。
///
/// 预填规则(对齐影子「词条取剪贴板」):
///   1. 词条 ← 剪贴板(看起来像一个词才用:1~32 字、无换行);
///   2. 编码 ← hint(用户打了一半拼音时);否则由词条自动拆简拼;
///   3. 位置 ← 1。
pub(crate) fn open_with_hint(hint: &str) {
    let st = global_state();
    let existing = { st.lock().unwrap().borrow().hwnd };
    let hwnd = match existing {
        Some(h) if unsafe { IsWindow(h).as_bool() } => h,
        _ => match create_window() {
            Ok(h) => {
                st.lock().unwrap().borrow_mut().hwnd = Some(h);
                h
            }
            Err(e) => {
                #[cfg(feature = "dllentry_log")]
                crate::com_class_factory::log_dll_entry(&format!("QuickAdd: create FAIL {e:?}"));
                let _ = e;
                return;
            }
        },
    };

    let word = clipboard_text().filter(|s| word_like(s)).unwrap_or_default();
    let mut code: String = hint.chars().filter(|c| c.is_ascii_alphabetic()).collect::<String>().to_lowercase();
    if code.is_empty() {
        code = crate::ffi::auto_code(&word).unwrap_or_default();
    }

    set_text(hwnd, IDC_QA_WORD, &word);
    set_text(hwnd, IDC_QA_CODE, &code);
    set_text(hwnd, IDC_QA_POS, "1");
    // 提示里带当前热键配置:用户改过组合/关掉热键后,这里必须说新组合(不写死 Ctrl+Alt+E)。
    let hk = crate::hotkey::load_quick_add();
    if word.is_empty() {
        // 2026-11-25 修正文案:输入框在说明文字的**上方**,写「下面」会让用户找不到入口。
        let msg = if hk.enabled {
            format!("先复制词条(选中 → Ctrl+C)再按 {};或直接在上方输入。", hk.describe())
        } else {
            "先复制词条(选中 → Ctrl+C),再从托盘菜单「造词」打开;或直接在上方输入。".to_string()
        };
        set_text(hwnd, IDC_QA_STATUS, &msg);
    } else if code.is_empty() {
        set_text(hwnd, IDC_QA_STATUS, "词条已从剪贴板带入;编码推导失败,请手填。");
    } else {
        set_text(hwnd, IDC_QA_STATUS, "编码已自动拆分,可直接改;位置 = 候选第几位。");
    }
    set_checked(hwnd, IDC_QA_MORE, false);
    {
        let g = st.lock().unwrap();
        let mut s = g.borrow_mut();
        s.code_auto = true;
        s.setting = false;
    }

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
    focus_and_select(hwnd, IDC_QA_WORD);
}

/// DLL 卸载/停用时销毁窗口。子类化随控件销毁自动解除。
pub(crate) fn destroy() {
    let st = global_state();
    let hwnd = st.lock().unwrap().borrow_mut().hwnd.take();
    if let Some(h) = hwnd {
        unsafe { let _ = DestroyWindow(h); }
    }
}

/// 独立进程入口:`prisir_tsfsvc --quick-add`。
///
/// **为什么需要它**:热键 `Ctrl+Alt+E` 会被别的软件抢(测试机上就撞了),
/// 而 TSF 侧的按键处理只能在输入法宿主进程里跑。这条通道不依赖任何热键,
/// 也不依赖 TSF 宿主 —— 直接起窗口 + 自带消息循环,写的是同一张 user_word 表。
pub fn run_standalone() -> i32 {
    // 单实例:已有造词窗口(不管是本进程还是 TSF 宿主里那个)就把它提到前面,
    // 不再开第二个 —— 两个窗口同时写同一张 user_word 表容易让人以为「加了两遍」。
    {
        let cls = wcs(WIN_CLASS);
        if let Ok(h) = unsafe { FindWindowW(PCWSTR(cls.as_ptr()), PCWSTR::null()) } {
            if !h.0.is_null() {
                unsafe {
                    let _ = ShowWindow(h, SW_SHOW);
                    let _ = SetForegroundWindow(h);
                }
                return 0;
            }
        }
    }
    STANDALONE.store(true, Ordering::SeqCst);
    open_with_hint("");
    let hwnd = global_state().lock().unwrap().borrow().hwnd;
    let Some(hwnd) = hwnd.filter(|h| unsafe { IsWindow(*h).as_bool() }) else {
        eprintln!("[quick-add] 窗口创建失败");
        return 1;
    };
    if get_text(hwnd, IDC_QA_WORD).is_empty() {
        set_text(
            hwnd,
            IDC_QA_STATUS,
            "先复制词条(选中 → Ctrl+C)再打开本窗口;或直接输入。",
        );
    }    unsafe {
        let mut msg = MSG::default();
        loop {
            let r = GetMessageW(&mut msg, HWND::default(), 0, 0);
            if r.0 <= 0 {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    0
}

// ── 窗口构建 ────────────────────────────────────────────────────────────────

fn register_class() -> Result<()> {
    let hinst = unsafe { GetModuleHandleW(None) }?;
    let class_name = wcs(WIN_CLASS);
    let mut existing = WNDCLASSEXW::default();
    if unsafe { GetClassInfoExW(hinst, PCWSTR(class_name.as_ptr()), &mut existing) }.is_ok() {
        return Ok(());
    }
    let hinst_h: HINSTANCE = hinst.into();
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinst_h,
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }?,
        hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut _),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    if unsafe { RegisterClassExW(&wc) } == 0 {
        return Err(Error::from_win32());
    }
    Ok(())
}

fn create_window() -> Result<HWND> {
    register_class()?;
    let hinst: HINSTANCE = unsafe { GetModuleHandleW(None) }?.into();
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
    let ex = WS_EX_TOOLWINDOW | WS_EX_TOPMOST;
    let mut rc = RECT { left: 0, top: 0, right: CLIENT_W, bottom: CLIENT_H };
    unsafe { AdjustWindowRectEx(&mut rc, style, false, ex)? };
    let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
    let (x, y) = anchor_near_cursor(w, h);
    let class_name = wcs(WIN_CLASS);
    let title = wcs(WIN_TITLE);
    unsafe {
        CreateWindowExW(
            ex, PCWSTR(class_name.as_ptr()), PCWSTR(title.as_ptr()), style,
            x, y, w, h, HWND::default(), HMENU::default(), hinst, None,
        )
    }
}

/// 光标附近弹窗:水平居中于光标、略偏下,并夹在显示器工作区内(多屏/任务栏都不越界)。
fn anchor_near_cursor(w: i32, h: i32) -> (i32, i32) {
    unsafe {
        let mut pt = POINT::default();
        if GetCursorPos(&mut pt).is_err() {
            return (200, 200);
        }
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return (pt.x, pt.y + 24);
        }
        let work = mi.rcWork;
        let x = (pt.x - w / 2).clamp(work.left, (work.right - w).max(work.left));
        let y = (pt.y + 24).clamp(work.top, (work.bottom - h).max(work.top));
        (x, y)
    }
}

fn create_children(hwnd: HWND) {
    let hinst: HINSTANCE = unsafe { GetModuleHandleW(None) }.unwrap_or_default().into();
    let font = cached_font();

    let mk_label = |id: u16, text: &str, x: i32, y: i32, w: i32, h: i32| unsafe {
        let cls = wcs("STATIC");
        let t = wcs(text);
        let h = CreateWindowExW(
            WINDOW_EX_STYLE(0), PCWSTR(cls.as_ptr()), PCWSTR(t.as_ptr()),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
            x, y, w, h, hwnd, HMENU(id as usize as *mut _), hinst, None,
        ).ok();
        if let Some(c) = h {
            let _ = SendMessageW(c, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        }
    };
    let mk_edit = |id: u16, x: i32, y: i32, w: i32, extra: u32| unsafe {
        let cls = wcs("EDIT");
        let h = CreateWindowExW(
            WS_EX_CLIENTEDGE, PCWSTR(cls.as_ptr()), PCWSTR::null(),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | (ES_AUTOHSCROLL as u32) | extra),
            x, y, w, ROW_H, hwnd, HMENU(id as usize as *mut _), hinst, None,
        ).ok();
        if let Some(c) = h {
            let _ = SendMessageW(c, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
            // Enter/Esc 必须能被我们接住:单行 EDIT 会自己消费这两个键,父窗口收不到 WM_KEYDOWN。
            let _ = SetWindowSubclass(c, Some(edit_subclass), 1, 0);
        }
    };
    let mk_btn = |id: u16, text: &str, x: i32, y: i32, w: i32, style: u32| unsafe {
        let cls = wcs("BUTTON");
        let t = wcs(text);
        let h = CreateWindowExW(
            WINDOW_EX_STYLE(0), PCWSTR(cls.as_ptr()), PCWSTR(t.as_ptr()),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | style),
            x, y, w, 30, hwnd, HMENU(id as usize as *mut _), hinst, None,
        ).ok();
        if let Some(c) = h {
            let _ = SendMessageW(c, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        }
    };

    let x_edit = MARGIN + LABEL_W;
    let w_edit = CLIENT_W - MARGIN * 2 - LABEL_W;
    let y_word = 14;
    let y_code = y_word + ROW_H + GAP;
    let y_pos = y_code + ROW_H + GAP;
    // 状态行独占整行(2 行高,48px —— 实测 2 行需 44.6px),按钮再单独一行 ——
    // 长提示不再被按钮/窗口高度裁掉。
    let y_status = y_pos + ROW_H + 14;
    let status_h = 48;
    let y_bar = y_status + status_h + 8;
    // 防回归:按钮 30 高 + 底部 10 留白必须落在客户区内。改 CLIENT_H/status_h 时这里会炸。
    debug_assert!(y_bar + 30 + 10 <= CLIENT_H, "造词窗口布局超出客户区高度");

    mk_label(IDC_QA_L_WORD, "词条", MARGIN, y_word, LABEL_W, ROW_H);
    mk_edit(IDC_QA_WORD, x_edit, y_word, w_edit, 0);

    mk_label(IDC_QA_L_CODE, "编码", MARGIN, y_code, LABEL_W, ROW_H);
    mk_edit(IDC_QA_CODE, x_edit, y_code, 200, 0);
    mk_label(IDC_QA_L_HINT, "自动拆简拼,可手改", x_edit + 208, y_code, w_edit - 208, ROW_H);

    mk_label(IDC_QA_L_POS, "位置", MARGIN, y_pos, LABEL_W, ROW_H);
    mk_edit(IDC_QA_POS, x_edit, y_pos, 56, ES_NUMBER as u32);
    mk_label(IDC_QA_L_HINT2, "候选第几位(1 起)", x_edit + 64, y_pos, 140, ROW_H);
    // 「继续造词」:连续加词不用反复唤窗(影子的同名勾选)。
    unsafe {
        let cls = wcs("BUTTON");
        let t = wcs("继续造词");
        if let Ok(c) = CreateWindowExW(
            WINDOW_EX_STYLE(0), PCWSTR(cls.as_ptr()), PCWSTR(t.as_ptr()),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | (BS_AUTOCHECKBOX as u32)),
            x_edit + 216, y_pos + 2, 120, 24, hwnd, HMENU(IDC_QA_MORE as usize as *mut _), hinst, None,
        ) {
            let _ = SendMessageW(c, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        }
    }

    mk_label(IDC_QA_STATUS, "", MARGIN, y_status, CLIENT_W - MARGIN * 2, status_h);
    let btn_w = 92;
    mk_btn(IDC_QA_SAVE, "保存", CLIENT_W - MARGIN - btn_w, y_bar, btn_w, BS_DEFPUSHBUTTON as u32);
    mk_btn(IDC_QA_CANCEL, "取消", CLIENT_W - MARGIN - btn_w * 2 - 8, y_bar, btn_w, BS_PUSHBUTTON as u32);
    // 保存/取消靠右,左下角放快捷键提示,不占状态行宽度。
    mk_label(IDC_QA_L_KEYS, "Enter 保存 · Esc 退出", MARGIN, y_bar + 6, 220, 24);
}

// ── 行为 ────────────────────────────────────────────────────────────────────

fn get_text(hwnd: HWND, id: u16) -> String {
    let Ok(ctl) = (unsafe { GetDlgItem(hwnd, id as i32) }) else { return String::new(); };
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(ctl, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn set_text(hwnd: HWND, id: u16, s: &str) {
    let Ok(ctl) = (unsafe { GetDlgItem(hwnd, id as i32) }) else { return; };
    let st = global_state();
    // **锁必须在 SetWindowTextW 之前释放**:SetWindowTextW 同步派发 EN_CHANGE,
    // 窗口过程会再取同一把锁 → 同一线程重入 std::sync::Mutex 直接死锁
    // (2026-11-25 实测:--quick-add 进程卡住、窗口创建了却永不显示)。
    let prev = {
        let guard = st.lock().unwrap();
        let mut g = guard.borrow_mut();
        let prev = g.setting;
        g.setting = true;
        prev
    };
    let t = wcs(s);
    unsafe { let _ = SetWindowTextW(ctl, PCWSTR(t.as_ptr())); }
    st.lock().unwrap().borrow_mut().setting = prev;
}

fn set_checked(hwnd: HWND, id: u16, on: bool) {
    if let Ok(ctl) = unsafe { GetDlgItem(hwnd, id as i32) } {
        unsafe {
            SendMessageW(
                ctl,
                BM_SETCHECK,
                WPARAM(if on { BST_CHECKED.0 as usize } else { BST_UNCHECKED.0 as usize }),
                LPARAM(0),
            );
        }
    }
}

fn is_checked(hwnd: HWND, id: u16) -> bool {
    unsafe { GetDlgItem(hwnd, id as i32) }
        .map(|c| unsafe { SendMessageW(c, BM_GETCHECK, WPARAM(0), LPARAM(0)) }.0 == BST_CHECKED.0 as isize)
        .unwrap_or(false)
}

fn focus_and_select(hwnd: HWND, id: u16) {
    if let Ok(ctl) = unsafe { GetDlgItem(hwnd, id as i32) } {
        unsafe {
            let _ = SetFocus(ctl);
            SendMessageW(ctl, EM_SETSEL, WPARAM(0), LPARAM(-1));
        }
    }
}

fn on_save(hwnd: HWND) {
    let word = get_text(hwnd, IDC_QA_WORD).trim().to_string();
    if word.is_empty() {
        set_text(hwnd, IDC_QA_STATUS, "词条为空:先复制中文词条,或直接在这里输入。");
        focus_and_select(hwnd, IDC_QA_WORD);
        return;
    }
    let mut code: String = get_text(hwnd, IDC_QA_CODE).to_lowercase();
    code.retain(|c| c.is_ascii_alphabetic());
    if code.is_empty() {
        code = crate::ffi::auto_code(&word).unwrap_or_default();
    }
    if code.is_empty() {
        set_text(hwnd, IDC_QA_STATUS, "推导不出编码(可能含生僻字),请手填编码。");
        focus_and_select(hwnd, IDC_QA_CODE);
        return;
    }
    let pos = get_text(hwnd, IDC_QA_POS).trim().parse::<i64>().unwrap_or(1).clamp(1, 50);

    if crate::ffi::user_dict_add_pos(&code, &word, pos) {
        set_text(hwnd, IDC_QA_STATUS, &format!("已加入:{word} → {code}(候选第 {pos} 位)"));
        if is_checked(hwnd, IDC_QA_MORE) {
            // 继续造词:清空词条/编码,位置保留(常连着加同一批同优先级的词)。
            set_text(hwnd, IDC_QA_WORD, "");
            set_text(hwnd, IDC_QA_CODE, "");
            global_state().lock().unwrap().borrow_mut().code_auto = true;
            focus_and_select(hwnd, IDC_QA_WORD);
        } else {
            hide_window();
        }
    } else {
        set_text(hwnd, IDC_QA_STATUS, "保存失败:引擎未就绪或词库不可写。");
    }
}

fn maybe_autofill_code(hwnd: HWND) {
    let st = global_state();
    // setting=true 表示是我们在写控件文本(不是用户在打字)→ 不必回填,也避免递归。
    let (auto, setting) = {
        let g = st.lock().unwrap();
        let b = g.borrow();
        (b.code_auto, b.setting)
    };
    if setting || !auto {
        return; // 程序回填中 / 用户手改过编码
    }
    let word = get_text(hwnd, IDC_QA_WORD).trim().to_string();
    if word.is_empty() {
        return;
    }
    if let Some(code) = crate::ffi::auto_code(&word) {
        if code != get_text(hwnd, IDC_QA_CODE) {
            set_text(hwnd, IDC_QA_CODE, &code);
        }
    }
}

fn hide_window() {
    let st = global_state();
    let hwnd = st.lock().unwrap().borrow().hwnd;
    if let Some(h) = hwnd {
        if STANDALONE.load(Ordering::SeqCst) {
            // 独立进程:窗口关了就该退出,否则消息循环永远挂着。
            unsafe {
                let _ = DestroyWindow(h);
                PostQuitMessage(0);
            }
            st.lock().unwrap().borrow_mut().hwnd = None;
        } else {
            unsafe { let _ = ShowWindow(h, SW_HIDE); }
        }
    }
}

fn clipboard_text() -> Option<String> {
    unsafe {
        use windows::Win32::System::DataExchange::*;
        use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
        use windows::Win32::System::Ole::CF_UNICODETEXT;
        if OpenClipboard(HWND::default()).is_err() {
            return None;
        }
        let mut out = None;
        if let Ok(h) = GetClipboardData(CF_UNICODETEXT.0 as u32) {
            let p = GlobalLock(HGLOBAL(h.0));
            if !p.is_null() {
                let s = CStr16::from_ptr(p as *const u16).to_string();
                let _ = GlobalUnlock(HGLOBAL(h.0));
                if !s.trim().is_empty() {
                    out = Some(s);
                }
            }
        }
        let _ = CloseClipboard();
        out
    }
}

/// 剪贴板内容是否「像一个词条」:1~32 字、不含换行/制表等控制字符。
/// 避免用户剪贴板里是整段文章/代码时被莫名填进来。
fn word_like(s: &str) -> bool {
    let n = s.chars().count();
    (1..=32).contains(&n) && !s.chars().any(|c| c.is_control())
}

/// 读 UTF-16 C 字符串的小工具(GetClipboardData 返回的是不透明指针)。
struct CStr16;
impl CStr16 {
    unsafe fn from_ptr(p: *const u16) -> String {
        let mut len = 0usize;
        while *p.add(len) != 0 {
            len += 1;
            if len > 8192 {
                break;
            }
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
    }
}

// ── 窗口过程 ────────────────────────────────────────────────────────────────

unsafe extern "system" fn edit_subclass(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if msg == WM_KEYDOWN {
        match wp.0 {
            VK_RETURN_U => {
                if let Ok(parent) = GetParent(hwnd) {
                    on_save(parent);
                }
                return LRESULT(0);
            }
            VK_ESCAPE_U => {
                hide_window();
                return LRESULT(0);
            }
            _ => {}
        }
    }
    DefSubclassProc(hwnd, msg, wp, lp)
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            create_children(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => {
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let brush = bg_brush();
            let _ = FillRect(HDC(wp.0 as *mut _), &rc, brush);
            LRESULT(1)
        }
        WM_CTLCOLORSTATIC => {
            let hdc = HDC(wp.0 as *mut _);
            // 提示行/状态行用弱色，主标签用正文色 —— 靠控件 ID 区分。
            let id = GetDlgCtrlID(HWND(lp.0 as *mut _));
            let color = match id as u16 {
                IDC_QA_L_HINT | IDC_QA_L_HINT2 | IDC_QA_L_KEYS => COL_HINT,
                IDC_QA_STATUS => COL_ACCENT,
                _ => COL_TEXT,
            };
            SetTextColor(hdc, COLORREF(color));
            SetBkMode(hdc, TRANSPARENT);
            LRESULT(bg_brush().0 as isize)
        }
        WM_CTLCOLOREDIT => {
            let hdc = HDC(wp.0 as *mut _);
            SetTextColor(hdc, COLORREF(COL_TEXT));
            LRESULT(edit_brush().0 as isize)
        }
        WM_COMMAND => {
            let id = (wp.0 & 0xFFFF) as u16;
            let notify = ((wp.0 >> 16) & 0xFFFF) as u32;
            match id {
                IDC_QA_SAVE => on_save(hwnd),
                IDC_QA_CANCEL => hide_window(),
                IDC_QA_WORD if notify == EN_CHANGE => maybe_autofill_code(hwnd),
                IDC_QA_CODE if notify == EN_CHANGE => {
                    // 用户手改编码 → 之后不再自动覆盖。
                    let st = global_state();
                    let g = st.lock().unwrap();
                    let mut s = g.borrow_mut();
                    if !s.setting {
                        s.code_auto = false;
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            hide_window();
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),    }
}

#[cfg(test)]
mod tests {
    use super::word_like;

    #[test]
    fn word_like_accepts_chinese_phrase() {
        assert!(word_like("北京大学"));
        assert!(word_like("灵犀输入法"));
    }

    #[test]
    fn word_like_rejects_paragraph_and_empty() {
        assert!(!word_like(""));
        assert!(!word_like("第一行\n第二行"));
        // 超过 32 字的段落不当词条
        assert!(!word_like(&"词".repeat(33)));
    }
}
