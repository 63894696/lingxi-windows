//! 关于 / 隐私政策 / 使用条款 / 开放 API 展示窗。
//!
//! 开放 API 页：正文可滚；底部状态单独一行；启用/关闭；端口输入框 + 应用按钮。

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
const WIN_H: i32 = 480;
const WIN_W_OPENAPI: i32 = 640;
const WIN_H_OPENAPI: i32 = 580;
const MARGIN: i32 = 20;

const IDC_PORT_EDIT: i32 = 4001;
const IDC_PORT_APPLY: i32 = 4002;
const IDC_ENABLE: i32 = 4003;
const IDC_DISABLE: i32 = 4004;

const COL_BG: u32 = 0x00FAF9F7;
const COL_TITLE: u32 = 0x00333333;
const COL_TEXT: u32 = 0x005A544D;
const COL_ACCENT: u32 = 0x00A9750A;
const COL_BTN: u32 = 0x00EFECE7;

fn wcs(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct AboutState {
    hwnd: Option<HWND>,
    key: String,
    dynamic_body: String,
    scroll_y: i32,
    hwnd_port_edit: Option<HWND>,
    hwnd_port_apply: Option<HWND>,
    hwnd_enable: Option<HWND>,
    hwnd_disable: Option<HWND>,
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
                scroll_y: 0,
                hwnd_port_edit: None,
                hwnd_port_apply: None,
                hwnd_enable: None,
                hwnd_disable: None,
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

pub(crate) fn show(key: &str) {
    let (title, body) = crate::about::lookup(key).unwrap_or(("关于", crate::about::ABOUT_BODY));
    open_window(&title, body, false, key.to_string())
}

pub(crate) fn show_dynamic(title: &str, body: &str) {
    open_window(title, body, true, "dynamic".into())
}

fn destroy_openapi_controls(s: &mut AboutState) {
    for h in [
        s.hwnd_port_edit.take(),
        s.hwnd_port_apply.take(),
        s.hwnd_enable.take(),
        s.hwnd_disable.take(),
    ]
    .into_iter()
    .flatten()
    {
        unsafe {
            let _ = DestroyWindow(h);
        }
    }
}

fn open_window(title: &str, body: &str, dynamic: bool, key: String) {
    let st = state();
    {
        let mut s = st.lock().unwrap();
        s.key = key.clone();
        s.dynamic_body = body.to_string();
        s.scroll_y = 0;
        if let Some(h) = s.hwnd {
            if unsafe { IsWindow(h) }.as_bool() {
                unsafe {
                    let _ = SetWindowTextW(h, PCWSTR(wcs(title).as_ptr()));
                    let _ = SetForegroundWindow(h);
                }
                if key == "openapi" {
                    ensure_openapi_controls(h, &mut s);
                    layout_openapi_controls(h, &s);
                } else {
                    destroy_openapi_controls(&mut s);
                }
                unsafe {
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
    let (ww, wh) = if key == "openapi" {
        (WIN_W_OPENAPI, WIN_H_OPENAPI)
    } else {
        (WIN_W, WIN_H)
    };
    let sw = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let sh = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    let x = (sw - ww) / 2;
    let y = (sh - wh) / 2;
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST,
            PCWSTR(cn.as_ptr()),
            PCWSTR(ti.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            x,
            y,
            ww,
            wh,
            None,
            None,
            hinst,
            None,
        )
    };
    if let Ok(h) = hwnd {
        {
            let mut s = st.lock().unwrap();
            s.hwnd = Some(h);
            if key == "openapi" {
                ensure_openapi_controls(h, &mut s);
                layout_openapi_controls(h, &s);
            }
        }
        unsafe {
            let _ = ShowWindow(h, SW_SHOW);
            let _ = UpdateWindow(h);
        }
    }
    let _ = dynamic;
}

fn client_size(hwnd: HWND) -> (i32, i32) {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    (rc.right - rc.left, rc.bottom - rc.top)
}

/// 开放 API 底部：状态行 + 启用行 + 端口行；其它页仅操作按钮。
fn footer_height(key: &str) -> i32 {
    if key == "openapi" {
        168
    } else if has_action_btn(key) {
        90
    } else {
        24
    }
}

fn btn_rect_at(client_h: i32) -> RECT {
    RECT {
        left: MARGIN,
        top: client_h - 54,
        right: MARGIN + 160,
        bottom: client_h - 20,
    }
}

fn ensure_openapi_controls(parent: HWND, s: &mut AboutState) {
    if s.hwnd_port_edit.is_some() {
        return;
    }
    let hinst = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
    let cfg = crate::open_api::load_config();
    unsafe {
        let btn = wcs("BUTTON");
        let edit = wcs("EDIT");
        let en = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(btn.as_ptr()),
            PCWSTR(wcs("启用").as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
            0,
            0,
            100,
            32,
            parent,
            HMENU(IDC_ENABLE as *mut _),
            hinst,
            None,
        )
        .ok();
        let dis = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(btn.as_ptr()),
            PCWSTR(wcs("关闭").as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
            0,
            0,
            100,
            32,
            parent,
            HMENU(IDC_DISABLE as *mut _),
            hinst,
            None,
        )
        .ok();
        let ed = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            PCWSTR(edit.as_ptr()),
            PCWSTR(wcs(&cfg.port.to_string()).as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_NUMBER as u32 | ES_LEFT as u32),
            0,
            0,
            100,
            28,
            parent,
            HMENU(IDC_PORT_EDIT as *mut _),
            hinst,
            None,
        )
        .ok();
        let ap = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(btn.as_ptr()),
            PCWSTR(wcs("应用").as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
            0,
            0,
            80,
            32,
            parent,
            HMENU(IDC_PORT_APPLY as *mut _),
            hinst,
            None,
        )
        .ok();
        s.hwnd_enable = en;
        s.hwnd_disable = dis;
        s.hwnd_port_edit = ed;
        s.hwnd_port_apply = ap;
    }
}

fn layout_openapi_controls(parent: HWND, s: &AboutState) {
    let (_cw, ch) = client_size(parent);
    // 自下而上留白：应用行 → 启用行 → 状态文字（paint）
    let y_port = ch - 52;
    let y_en = ch - 100;
    unsafe {
        if let Some(h) = s.hwnd_enable {
            let _ = SetWindowPos(h, HWND::default(), MARGIN, y_en, 100, 32, SWP_NOZORDER);
        }
        if let Some(h) = s.hwnd_disable {
            let _ = SetWindowPos(h, HWND::default(), MARGIN + 112, y_en, 100, 32, SWP_NOZORDER);
        }
        if let Some(h) = s.hwnd_port_edit {
            let _ = SetWindowPos(h, HWND::default(), MARGIN + 56, y_port, 120, 28, SWP_NOZORDER);
        }
        if let Some(h) = s.hwnd_port_apply {
            let _ = SetWindowPos(h, HWND::default(), MARGIN + 186, y_port, 80, 32, SWP_NOZORDER);
        }
    }
}

fn read_edit_text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; (len + 1) as usize];
        let n = GetWindowTextW(hwnd, &mut buf);
        if n <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

fn apply_port_from_edit() -> std::result::Result<(), String> {
    let st = state();
    let edit = st
        .lock()
        .unwrap()
        .hwnd_port_edit
        .ok_or_else(|| "无端口输入框".to_string())?;
    let text = read_edit_text(edit);
    let port: u16 = text
        .trim()
        .parse()
        .map_err(|_| "端口须为 1–65535 的数字".to_string())?;
    if port == 0 {
        return Err("端口须为 1–65535".into());
    }
    let mut cfg = crate::open_api::load_config();
    cfg.port = port;
    crate::open_api::save_config(&cfg).map_err(|e| e.to_string())?;
    Ok(())
}

fn point_in(r: &RECT, cx: i32, cy: i32) -> bool {
    cx >= r.left && cx < r.right && cy >= r.top && cy < r.bottom
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
        WM_SIZE => {
            let st = state();
            let s = st.lock().unwrap();
            if s.key == "openapi" {
                layout_openapi_controls(hwnd, &s);
            }
            drop(s);
            let _ = InvalidateRect(hwnd, None, true);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) as i16) as i32;
            let st = state();
            let mut s = st.lock().unwrap();
            s.scroll_y = (s.scroll_y - delta / 4).max(0);
            drop(s);
            let _ = InvalidateRect(hwnd, None, true);
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            match id {
                IDC_ENABLE => {
                    let mut cfg = crate::open_api::load_config();
                    cfg.enabled = true;
                    let _ = crate::open_api::save_config(&cfg);
                    let _ = InvalidateRect(hwnd, None, true);
                }
                IDC_DISABLE => {
                    let mut cfg = crate::open_api::load_config();
                    cfg.enabled = false;
                    let _ = crate::open_api::save_config(&cfg);
                    let _ = InvalidateRect(hwnd, None, true);
                }
                IDC_PORT_APPLY => {
                    match apply_port_from_edit() {
                        Ok(()) => {
                            let _ = SetWindowTextW(hwnd, PCWSTR(wcs("开放 API — 端口已应用").as_ptr()));
                            let _ = InvalidateRect(hwnd, None, true);
                        }
                        Err(e) => {
                            let t = format!("开放 API — {e}");
                            let _ = SetWindowTextW(hwnd, PCWSTR(wcs(&t).as_ptr()));
                        }
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            let st = state();
            let mut s = st.lock().unwrap();
            destroy_openapi_controls(&mut s);
            s.hwnd = None;
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
            let (_cw, ch) = client_size(hwnd);
            let btn = btn_rect_at(ch);
            if point_in(&btn, cx, cy) && has_action_btn(&key) {
                if key == "about" {
                    let url = wcs(crate::about::WEBSITE_URL);
                    let open = wcs("open");
                    let _ = ShellExecuteW(
                        None,
                        PCWSTR(open.as_ptr()),
                        PCWSTR(url.as_ptr()),
                        PCWSTR::null(),
                        PCWSTR::null(),
                        SW_SHOW,
                    );
                } else if key == "feedback" || key == "contact" {
                    crate::feedback::open_forum();
                } else if key == "dynamic" {
                    let body = st.lock().unwrap().dynamic_body.clone();
                    if copy_to_clipboard(&body) {
                        let _ = SetWindowTextW(hwnd, PCWSTR(wcs("详情 — 已复制到剪贴板").as_ptr()));
                    }
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn has_action_btn(key: &str) -> bool {
    key == "about" || key == "feedback" || key == "contact" || key == "dynamic"
}

fn action_btn_label(key: &str) -> &'static str {
    match key {
        "about" => "打开官网",
        "feedback" | "contact" => "打开论坛",
        "dynamic" => "复制内容",
        _ => "",
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
                Ok(h) if !h.is_invalid() => {
                    let p = GlobalLock(h);
                    if p.is_null() {
                        false
                    } else {
                        std::ptr::copy_nonoverlapping(units.as_ptr(), p as *mut u16, units.len());
                        let _ = GlobalUnlock(h);
                        SetClipboardData(CF_UNICODETEXT.0 as u32, HANDLE(h.0)).is_ok()
                    }
                }
                _ => false,
            }
        } else {
            false
        };
        let _ = CloseClipboard();
        ok
    }
}

fn draw_btn(hdc: HDC, btn: RECT, label: &str) {
    unsafe {
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
        DrawTextW(hdc, &mut lw, &mut lrc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }
}

fn paint(hwnd: HWND) {
    let (key, scroll_y, dynamic_body) = {
        let st = state();
        let s = st.lock().unwrap();
        (s.key.clone(), s.scroll_y, s.dynamic_body.clone())
    };
    let (title, body) = if key == "dynamic" {
        ("详情".to_string(), dynamic_body)
    } else {
        let (t, b) = crate::about::lookup(&key).unwrap_or(("关于", crate::about::ABOUT_BODY));
        (t.to_string(), b.to_string())
    };
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
            -20, 0, 0, 0, FW_BOLD.0 as i32, 0, 0, 0,
            DEFAULT_CHARSET.0 as u32, OUT_DEFAULT_PRECIS.0 as u32, CLIP_DEFAULT_PRECIS.0 as u32,
            CLEARTYPE_QUALITY.0 as u32, DEFAULT_PITCH.0 as u32, PCWSTR(wcs("Microsoft YaHei UI").as_ptr()),
        );
        let old = if !tf.is_invalid() { Some(SelectObject(hdc, tf)) } else { None };
        SetTextColor(hdc, COLORREF(COL_TITLE));
        let mut trc = RECT { left: MARGIN, top: MARGIN, right: rc.right - MARGIN, bottom: MARGIN + 30 };
        let mut t = wcs(&title);
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

        let foot = footer_height(&key);
        let body_bottom = rc.bottom - foot;

        if key == "openapi" {
            let cfg = crate::open_api::load_config();
            let status = format!(
                "当前：{}　·　仅 127.0.0.1　·　配置 %LOCALAPPDATA%\\Prisir\\IME\\open_api.json",
                if cfg.enabled { "已启用" } else { "已关闭" }
            );
            let sf = CreateFontW(
                -14, 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0,
                DEFAULT_CHARSET.0 as u32, OUT_DEFAULT_PRECIS.0 as u32, CLIP_DEFAULT_PRECIS.0 as u32,
                CLEARTYPE_QUALITY.0 as u32, DEFAULT_PITCH.0 as u32, PCWSTR(wcs("Microsoft YaHei UI").as_ptr()),
            );
            let olds = if !sf.is_invalid() { Some(SelectObject(hdc, sf)) } else { None };
            SetTextColor(hdc, COLORREF(COL_ACCENT));
            // 状态单独一行，在启用按钮上方
            let mut src = RECT {
                left: MARGIN,
                top: rc.bottom - 140,
                right: rc.right - MARGIN,
                bottom: rc.bottom - 112,
            };
            let mut sw = wcs(&status);
            DrawTextW(hdc, &mut sw, &mut src, DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS);
            // 「端口」标签（输入框左侧）
            let mut plab = RECT {
                left: MARGIN,
                top: rc.bottom - 52,
                right: MARGIN + 52,
                bottom: rc.bottom - 20,
            };
            let mut pw = wcs("端口");
            DrawTextW(hdc, &mut pw, &mut plab, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
            if let Some(o) = olds {
                SelectObject(hdc, o);
                let _ = DeleteObject(sf);
            }
        }

        let bf = CreateFontW(
            -15, 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0,
            DEFAULT_CHARSET.0 as u32, OUT_DEFAULT_PRECIS.0 as u32, CLIP_DEFAULT_PRECIS.0 as u32,
            CLEARTYPE_QUALITY.0 as u32, DEFAULT_PITCH.0 as u32, PCWSTR(wcs("Microsoft YaHei UI").as_ptr()),
        );
        let old2 = if !bf.is_invalid() { Some(SelectObject(hdc, bf)) } else { None };
        SetTextColor(hdc, COLORREF(COL_TEXT));
        let clip = RECT {
            left: MARGIN,
            top: MARGIN + 50,
            right: rc.right - MARGIN,
            bottom: body_bottom,
        };
        IntersectClipRect(hdc, clip.left, clip.top, clip.right, clip.bottom);
        let mut brc = RECT {
            left: MARGIN,
            top: MARGIN + 50 - scroll_y,
            right: rc.right - MARGIN,
            bottom: MARGIN + 50 - scroll_y + 4000,
        };
        let mut b = wcs(&body);
        DrawTextW(hdc, &mut b, &mut brc, DT_LEFT | DT_WORDBREAK);
        SelectClipRgn(hdc, HRGN::default());

        if has_action_btn(&key) {
            draw_btn(hdc, btn_rect_at(rc.bottom), action_btn_label(&key));
        }

        if let Some(o) = old2 {
            SelectObject(hdc, o);
            let _ = DeleteObject(bf);
        }
        let _ = EndPaint(hwnd, &ps);
    }
}
