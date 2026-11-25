//! 关于/使用说明/隐私/条款/开放API/问题反馈 展示窗(2026-09-04；2026-11-25 文案对齐 Linux)。
//!
//! **定位**:状态栏菜单点对应项弹出,显示 about.rs 正文。只读展示。
//!
//! **形态**:苹果风自绘(暖白底/雅黑),居中。正文自动换行。问题反馈页附「打开论坛」按钮。
//!
//! **崩溃预防(2026-09-04 词库窗教训)**:所有 DrawTextW 文本走 wcs() 补 `\0` 结尾,
//! windows-rs 0.58 对 `&mut Vec<u16>` 按 C 字符串(-1)处理,无 null 结尾越界读带崩宿主。

use std::sync::{Arc, Mutex, OnceLock};
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

const WIN_CLASS: &str = "PrisirAboutWin";
const WIN_W: i32 = 560;
/// 正文偏长时原先 460 裁掉末行；再加一行高（约 24px）。
const WIN_H: i32 = 484;
const MARGIN: i32 = 20;

// 苹果风配色(BGR 0x00BBGGRR),与状态栏/词库窗一致。
const COL_BG: u32 = 0x00FAF9F7;
const COL_TITLE: u32 = 0x00333333;
const COL_TEXT: u32 = 0x005A544D;
const COL_ACCENT: u32 = 0x00A9750A;
const COL_BTN: u32 = 0x00EFECE7;

/// UTF-16 buffer 带 null 结尾(防 DrawTextW 越界,见 userdict_window.rs 教训)。
fn wcs(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct AboutState {
    hwnd: Option<HWND>,
    key: String,           // about | help | privacy | terms | openapi | contact | feedback | dynamic
    dynamic_body: String,  // key=="dynamic" 时的正文
    dynamic_title: String, // key=="dynamic" 时的标题
}
static ABOUT_STATE: OnceLock<Arc<Mutex<AboutState>>> = OnceLock::new();
unsafe impl Send for AboutState {}
unsafe impl Sync for AboutState {}

fn state() -> Arc<Mutex<AboutState>> {
    ABOUT_STATE
        .get_or_init(|| {
            Arc::new(Mutex::new(AboutState {
                hwnd: None,
                key: "about".into(),
                dynamic_body: String::new(),
                dynamic_title: String::new(),
            }))
        })
        .clone()
}

fn register_class() -> Result<()> {
    let hinst = unsafe { GetModuleHandleW(None) }?;
    let cn = wcs(WIN_CLASS);
    let mut existing = WNDCLASSEXW::default();
    if unsafe { GetClassInfoExW(hinst, PCWSTR(cn.as_ptr()), &mut existing) }.is_ok() {
        return Ok(());
    }
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(about_wnd_proc),
        hInstance: hinst.into(),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }?,
        hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut _),
        lpszClassName: PCWSTR(cn.as_ptr()),
        ..Default::default()
    };
    if unsafe { RegisterClassExW(&wc) } == 0 {
        return Err(Error::from_win32());
    }
    Ok(())
}

/// 打开/聚焦 关于窗。key: help|about|privacy|terms|openapi|feedback|contact。
pub(crate) fn show(key: &str) {
    let (title, body) = crate::about::lookup(key).unwrap_or(("关于", crate::about::ABOUT_BODY));
    let key_owned = key.to_string();
    open_window(&title, body, false, key_owned)
}

/// 打开/聚焦 动态内容窗(任意提示文本,带「复制文本」按钮)。
pub(crate) fn show_dynamic(title: &str, body: &str) {
    open_window(title, body, true, "dynamic".into())
}

fn needs_action_btn(key: &str) -> bool {
    matches!(key, "contact" | "feedback" | "dynamic")
}

fn is_feedback_key(key: &str) -> bool {
    matches!(key, "contact" | "feedback")
}

/// 公共开窗逻辑。
fn open_window(title: &str, body: &str, dynamic: bool, key: String) {
    let st = state();
    {
        let mut s = st.lock().unwrap();
        s.key = key;
        s.dynamic_body = body.to_string();
        s.dynamic_title = title.to_string();
        if let Some(h) = s.hwnd {
            if unsafe { IsWindow(h) }.as_bool() {
                unsafe {
                    let _ = SetWindowTextW(h, PCWSTR(wcs(title).as_ptr()));
                    let _ = SetForegroundWindow(h);
                    let _ = InvalidateRect(h, None, true);
                }
                return;
            }
            s.hwnd = None;
        }
    }
    if register_class().is_err() {
        return;
    }
    let hinst = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
    let cn = wcs(WIN_CLASS);
    let ti = wcs(title);
    let sw = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let sh = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    let x = (sw - WIN_W) / 2;
    let y = (sh - WIN_H) / 2;
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST,
            PCWSTR(cn.as_ptr()),
            PCWSTR(ti.as_ptr()),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            x,
            y,
            WIN_W,
            WIN_H,
            None,
            None,
            hinst,
            None,
        )
    };
    if let Ok(h) = hwnd {
        st.lock().unwrap().hwnd = Some(h);
        unsafe {
            let _ = ShowWindow(h, SW_SHOW);
            let _ = UpdateWindow(h);
        }
    }
    let _ = dynamic;
}

unsafe extern "system" fn about_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            state().lock().unwrap().hwnd = None;
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 == VK_ESCAPE.0 as usize {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let st = state();
            let key = st.lock().unwrap().key.clone();
            let cy = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let cx = (lparam.0 & 0xFFFF) as i16 as i32;
            let hit = |r: RECT| cx >= r.left && cx < r.right && cy >= r.top && cy < r.bottom;
            if is_feedback_key(&key) {
                if hit(btn_forum_rect()) {
                    let url = wcs(crate::about::FORUM_URL);
                    let open = wcs("open");
                    let _ = ShellExecuteW(
                        None,
                        PCWSTR(open.as_ptr()),
                        PCWSTR(url.as_ptr()),
                        PCWSTR::null(),
                        PCWSTR::null(),
                        SW_SHOW,
                    );
                } else if hit(btn_diag_rect()) {
                    crate::feedback::feedback_and_open();
                }
            } else if key == "dynamic" && hit(btn_rect()) {
                let (body, title) = {
                    let g = st.lock().unwrap();
                    (g.dynamic_body.clone(), g.dynamic_title.clone())
                };
                if copy_to_clipboard(&body) {
                    let msg = format!("{title} — 已复制到剪贴板");
                    unsafe {
                        let _ = SetWindowTextW(hwnd, PCWSTR(wcs(&msg).as_ptr()));
                    }
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn btn_rect() -> RECT {
    // dynamic「复制文本」单按钮
    RECT {
        left: MARGIN,
        top: WIN_H - 90,
        right: MARGIN + 140,
        bottom: WIN_H - 90 + 34,
    }
}

fn btn_forum_rect() -> RECT {
    RECT {
        left: MARGIN,
        top: WIN_H - 90,
        right: MARGIN + 120,
        bottom: WIN_H - 90 + 34,
    }
}

fn btn_diag_rect() -> RECT {
    RECT {
        left: MARGIN + 132,
        top: WIN_H - 90,
        right: MARGIN + 132 + 140,
        bottom: WIN_H - 90 + 34,
    }
}

fn copy_to_clipboard(text: &str) -> bool {
    unsafe {
        use windows::Win32::System::DataExchange::*;
        use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
        use windows::Win32::System::Ole::CF_UNICODETEXT;
        if OpenClipboard(HWND::default()).is_err() {
            return false;
        }
        let ok = if EmptyClipboard().is_ok() {
            let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes = units.len() * 2;
            match GlobalAlloc(GMEM_MOVEABLE, bytes) {
                Ok(h) => {
                    let p = GlobalLock(h);
                    if p.is_null() {
                        false
                    } else {
                        std::ptr::copy_nonoverlapping(
                            units.as_ptr() as *const u8,
                            p as *mut u8,
                            bytes,
                        );
                        let _ = GlobalUnlock(h);
                        SetClipboardData(CF_UNICODETEXT.0 as u32, HANDLE(h.0)).is_ok()
                    }
                }
                Err(_) => false,
            }
        } else {
            false
        };
        let _ = CloseClipboard();
        ok
    }
}

fn paint(hwnd: HWND) {
    let key = state().lock().unwrap().key.clone();
    let (title, body) = if key == "dynamic" {
        let g = state();
        let g = g.lock().unwrap();
        (g.dynamic_title.clone(), g.dynamic_body.clone())
    } else {
        let (t, b) = crate::about::lookup(&key).unwrap_or(("关于", crate::about::ABOUT_BODY));
        (t.to_string(), b.to_string())
    };
    let title: &str = &title;
    let body: &str = &body;
    let mut ps = PAINTSTRUCT::default();
    unsafe {
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let bg = CreateSolidBrush(COLORREF(COL_BG));
        FillRect(hdc, &rc, bg);
        let _ = DeleteObject(bg);
        SetBkMode(hdc, TRANSPARENT);

        let tf = CreateFontW(
            -20,
            0,
            0,
            0,
            FW_BOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            CLEARTYPE_QUALITY.0 as u32,
            DEFAULT_PITCH.0 as u32,
            PCWSTR(wcs("Microsoft YaHei UI").as_ptr()),
        );
        let old = if !tf.is_invalid() {
            Some(SelectObject(hdc, tf))
        } else {
            None
        };
        SetTextColor(hdc, COLORREF(COL_TITLE));
        let mut trc = RECT {
            left: MARGIN,
            top: MARGIN,
            right: rc.right - MARGIN,
            bottom: MARGIN + 30,
        };
        let mut t = wcs(title);
        DrawTextW(hdc, &mut t, &mut trc, DT_LEFT | DT_SINGLELINE);
        let sep = CreatePen(PS_SOLID, 1, COLORREF(0x00ECE9E4));
        let op = SelectObject(hdc, sep);
        let _ = MoveToEx(hdc, MARGIN, MARGIN + 38, None);
        let _ = LineTo(hdc, rc.right - MARGIN, MARGIN + 38);
        SelectObject(hdc, op);
        let _ = DeleteObject(sep);
        if let Some(o) = old {
            SelectObject(hdc, o);
            let _ = DeleteObject(tf);
        }

        let bf = CreateFontW(
            -14,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            CLEARTYPE_QUALITY.0 as u32,
            DEFAULT_PITCH.0 as u32,
            PCWSTR(wcs("Microsoft YaHei UI").as_ptr()),
        );
        let old2 = if !bf.is_invalid() {
            Some(SelectObject(hdc, bf))
        } else {
            None
        };
        SetTextColor(hdc, COLORREF(COL_TEXT));
        let body_bottom = if needs_action_btn(&key) {
            btn_rect().top - 12
        } else {
            rc.bottom - MARGIN
        };
        let mut brc = RECT {
            left: MARGIN,
            top: MARGIN + 50,
            right: rc.right - MARGIN,
            bottom: body_bottom,
        };
        let mut b = wcs(body);
        DrawTextW(hdc, &mut b, &mut brc, DT_LEFT | DT_WORDBREAK);

        if needs_action_btn(&key) {
            let draw_btn = |btn: RECT, label: &str| {
                let bb = CreateSolidBrush(COLORREF(COL_BTN));
                let mut br = btn;
                FillRect(hdc, &mut br, bb);
                let _ = DeleteObject(bb);
                let pen = CreatePen(PS_SOLID, 1, COLORREF(COL_ACCENT));
                let opn = SelectObject(hdc, pen);
                let obr = SelectObject(hdc, GetStockObject(NULL_BRUSH));
                let _ = RoundRect(hdc, btn.left, btn.top, btn.right, btn.bottom, 8, 8);
                SelectObject(hdc, obr);
                SelectObject(hdc, opn);
                let _ = DeleteObject(pen);
                SetTextColor(hdc, COLORREF(COL_ACCENT));
                let mut lw = wcs(label);
                let mut lrc = btn;
                DrawTextW(
                    hdc,
                    &mut lw,
                    &mut lrc,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
            };
            if is_feedback_key(&key) {
                draw_btn(btn_forum_rect(), "打开论坛");
                draw_btn(btn_diag_rect(), "打包诊断");
            } else {
                draw_btn(btn_rect(), "复制文本");
            }
        }

        if let Some(o) = old2 {
            SelectObject(hdc, o);
            let _ = DeleteObject(bf);
        }
        let _ = EndPaint(hwnd, &ps);
    }
}
