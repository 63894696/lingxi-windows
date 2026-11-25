//! 开放 API 设置窗 — 版面对齐 about_window（苹果风自绘）。
//!
//! 仅保留：说明文字、启用开关、端口、应用。不展示配置路径 / curl 等技术内容。
//! 配置写 `%LOCALAPPDATA%\Prisir\IME\open_api.json`，并启停本进程内 HTTP 监听。

use std::sync::{Arc, Mutex, OnceLock};
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{BST_CHECKED, BST_UNCHECKED};
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::*;

const WIN_CLASS: &str = "PrisirOpenApiWin";
const WIN_TITLE: &str = "开放 API";
const WIN_W: i32 = 480;
const WIN_H: i32 = 300;
const MARGIN: i32 = 20;
const ROW_H: i32 = 28;

const IDC_CHECK: u16 = 4101;
const IDC_PORT: u16 = 4102;

// 与 about_window / status_bar 统一（BGR）
const COL_BG: u32 = 0x00FAF9F7;
const COL_TITLE: u32 = 0x00333333;
const COL_TEXT: u32 = 0x005A544D;
const COL_HINT: u32 = 0x008C857C;
const COL_ACCENT: u32 = 0x00A9750A;
const COL_BTN: u32 = 0x00EFECE7;

fn wcs(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct WinState {
    hwnd: Option<HWND>,
    /// 底部状态/结果提示（自绘，无 STATIC 灰底）
    status: String,
}
unsafe impl Send for WinState {}
unsafe impl Sync for WinState {}

static STATE: OnceLock<Arc<Mutex<WinState>>> = OnceLock::new();

fn state() -> Arc<Mutex<WinState>> {
    STATE
        .get_or_init(|| {
            Arc::new(Mutex::new(WinState {
                hwnd: None,
                status: String::new(),
            }))
        })
        .clone()
}

/// 打开/聚焦开放 API 设置窗。
pub(crate) fn show() {
    let st = state();
    {
        let mut g = st.lock().unwrap();
        if let Some(h) = g.hwnd {
            if unsafe { IsWindow(h) }.as_bool() {
                load_into_ui(h);
                refresh_status(h);
                unsafe {
                    let _ = ShowWindow(h, SW_SHOW);
                    let _ = SetForegroundWindow(h);
                    let _ = InvalidateRect(h, None, true);
                }
                return;
            }
            g.hwnd = None;
        }
    }
    if register_class().is_err() {
        return;
    }
    let Ok(hwnd) = create_window() else {
        return;
    };
    create_controls(hwnd);
    load_into_ui(hwnd);
    refresh_status(hwnd);
    st.lock().unwrap().hwnd = Some(hwnd);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
        let _ = SetForegroundWindow(hwnd);
    }
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
        lpfnWndProc: Some(wnd_proc),
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

fn create_window() -> Result<HWND> {
    // 与 about_window 完全同款边框：普通标题栏 + 系统关闭键（勿加 WS_EX_TOOLWINDOW，
    // 否则会出现窄标题栏/不同样式的 ×）。
    let hinst = unsafe { GetModuleHandleW(None) }?;
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
    let ex = WS_EX_TOPMOST;
    let sw = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let sh = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    let x = (sw - WIN_W) / 2;
    let y = (sh - WIN_H) / 2;
    let cn = wcs(WIN_CLASS);
    let ti = wcs(WIN_TITLE);
    unsafe {
        CreateWindowExW(
            ex,
            PCWSTR(cn.as_ptr()),
            PCWSTR(ti.as_ptr()),
            style,
            x,
            y,
            WIN_W,
            WIN_H,
            None,
            None,
            hinst,
            None,
        )
    }
}

fn create_controls(hwnd: HWND) {
    let hinst: HINSTANCE = unsafe { GetModuleHandleW(None) }.unwrap_or_default().into();
    let font = unsafe {
        CreateFontW(
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
        )
    };

    // 控件区：说明下方。标题/说明/状态均自绘，避免 STATIC 灰底框。
    let y_check = 118;
    let y_port = 156;

    unsafe {
        let cls = wcs("BUTTON");
        let t = wcs("启用开放 API");
        if let Ok(c) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(cls.as_ptr()),
            PCWSTR(t.as_ptr()),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | (BS_AUTOCHECKBOX as u32)),
            MARGIN,
            y_check,
            200,
            ROW_H,
            hwnd,
            HMENU(IDC_CHECK as usize as *mut _),
            hinst,
            None,
        ) {
            let _ = SendMessageW(c, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        }

        let cls = wcs("EDIT");
        if let Ok(c) = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            PCWSTR(cls.as_ptr()),
            PCWSTR::null(),
            WINDOW_STYLE(
                WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | (ES_NUMBER as u32) | (ES_AUTOHSCROLL as u32),
            ),
            MARGIN + 90,
            y_port,
            100,
            ROW_H,
            hwnd,
            HMENU(IDC_PORT as usize as *mut _),
            hinst,
            None,
        ) {
            let _ = SendMessageW(c, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        }
    }
}

fn child(hwnd: HWND, id: u16) -> Option<HWND> {
    unsafe { GetDlgItem(hwnd, id as i32) }.ok()
}

fn get_text(hwnd: HWND, id: u16) -> String {
    let Some(h) = child(hwnd, id) else {
        return String::new();
    };
    unsafe {
        let n = GetWindowTextLengthW(h);
        if n <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; (n + 1) as usize];
        let got = GetWindowTextW(h, &mut buf);
        if got <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..got as usize])
    }
}

fn set_port_text(hwnd: HWND, port: u16) {
    if let Some(h) = child(hwnd, IDC_PORT) {
        unsafe {
            let _ = SetWindowTextW(h, PCWSTR(wcs(&port.to_string()).as_ptr()));
        }
    }
}

fn load_into_ui(hwnd: HWND) {
    let cfg = crate::open_api::load_config();
    if let Some(chk) = child(hwnd, IDC_CHECK) {
        unsafe {
            let _ = SendMessageW(
                chk,
                BM_SETCHECK,
                WPARAM(if cfg.enabled {
                    BST_CHECKED.0 as usize
                } else {
                    BST_UNCHECKED.0 as usize
                }),
                LPARAM(0),
            );
        }
    }
    set_port_text(hwnd, cfg.port);
}

fn refresh_status(hwnd: HWND) {
    let cfg = crate::open_api::load_config();
    let up = crate::open_api::probe_health(cfg.port) || crate::open_api::is_listening();
    let text = if !cfg.enabled {
        "当前：已关闭".to_string()
    } else if up {
        format!("当前：已开启 · 端口 {} · 运行中", cfg.port)
    } else {
        format!("当前：已开启 · 端口 {} · 未在监听", cfg.port)
    };
    state().lock().unwrap().status = text;
    unsafe {
        let _ = InvalidateRect(hwnd, None, true);
    }
}

fn set_status(hwnd: HWND, msg: &str) {
    state().lock().unwrap().status = msg.to_string();
    unsafe {
        let _ = InvalidateRect(hwnd, None, true);
    }
}

fn on_apply(hwnd: HWND) {
    let enabled = child(hwnd, IDC_CHECK)
        .map(|chk| {
            let v = unsafe { SendMessageW(chk, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 };
            v == BST_CHECKED.0 as isize
        })
        .unwrap_or(false);
    let port: u16 = match get_text(hwnd, IDC_PORT).parse() {
        Ok(p) if (1024..=65535).contains(&p) => p,
        _ => {
            set_status(hwnd, "端口须在 1024–65535");
            return;
        }
    };
    let cfg = crate::open_api::OpenApiConfig {
        enabled,
        port,
        bind: crate::open_api::DEFAULT_BIND.to_string(),
    };
    set_status(hwnd, "正在应用…");
    match crate::open_api::apply_config(&cfg) {
        Ok(_) => {
            if enabled {
                if crate::open_api::probe_health(port) || crate::open_api::is_listening() {
                    set_status(hwnd, &format!("已开启 · 端口 {port} · 运行中"));
                } else {
                    set_status(hwnd, &format!("已保存启用 · 端口 {port}"));
                }
            } else {
                set_status(hwnd, "已关闭");
            }
        }
        Err(e) => set_status(hwnd, &format!("未能应用：{e}")),
    }
}

fn apply_btn_rect() -> RECT {
    RECT {
        left: WIN_W - MARGIN - 100,
        top: WIN_H - 70,
        right: WIN_W - MARGIN,
        bottom: WIN_H - 70 + 34,
    }
}

fn paint(hwnd: HWND) {
    let status = state().lock().unwrap().status.clone();
    let mut ps = PAINTSTRUCT::default();
    unsafe {
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let bg = CreateSolidBrush(COLORREF(COL_BG));
        FillRect(hdc, &rc, bg);
        let _ = DeleteObject(bg);
        SetBkMode(hdc, TRANSPARENT);

        // 标题
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
            bottom: MARGIN + 28,
        };
        let mut t = wcs("开放 API");
        DrawTextW(hdc, &mut t, &mut trc, DT_LEFT | DT_SINGLELINE);

        let sep = CreatePen(PS_SOLID, 1, COLORREF(0x00ECE9E4));
        let op = SelectObject(hdc, sep);
        let _ = MoveToEx(hdc, MARGIN, MARGIN + 36, None);
        let _ = LineTo(hdc, rc.right - MARGIN, MARGIN + 36);
        SelectObject(hdc, op);
        let _ = DeleteObject(sep);
        if let Some(o) = old {
            SelectObject(hdc, o);
            let _ = DeleteObject(tf);
        }

        // 说明（无灰底框）
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
        let mut brc = RECT {
            left: MARGIN,
            top: MARGIN + 48,
            right: rc.right - MARGIN,
            bottom: 110,
        };
        let mut body = wcs(
            "供本机 AI Agent 按习惯协助配置输入法。默认关闭；开启后仅在本机生效。",
        );
        DrawTextW(hdc, &mut body, &mut brc, DT_LEFT | DT_WORDBREAK);

        // 「监听端口」标签
        SetTextColor(hdc, COLORREF(COL_TEXT));
        let mut prc = RECT {
            left: MARGIN,
            top: 160,
            right: MARGIN + 88,
            bottom: 160 + 24,
        };
        let mut pl = wcs("监听端口");
        DrawTextW(hdc, &mut pl, &mut prc, DT_LEFT | DT_VCENTER | DT_SINGLELINE);

        // 提示
        SetTextColor(hdc, COLORREF(COL_HINT));
        let mut hrc = RECT {
            left: MARGIN,
            top: 192,
            right: rc.right - MARGIN,
            bottom: 214,
        };
        let mut hint = wcs("默认 23331，可改；改完后点「应用」。");
        DrawTextW(hdc, &mut hint, &mut hrc, DT_LEFT | DT_SINGLELINE);

        // 状态行
        SetTextColor(hdc, COLORREF(COL_TEXT));
        let mut src = RECT {
            left: MARGIN,
            top: WIN_H - 70,
            right: apply_btn_rect().left - 12,
            bottom: WIN_H - 70 + 34,
        };
        let mut st = wcs(&status);
        DrawTextW(
            hdc,
            &mut st,
            &mut src,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );

        // 「应用」按钮（与关于窗同款自绘）
        let btn = apply_btn_rect();
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
        let mut lw = wcs("应用");
        let mut lrc = btn;
        DrawTextW(
            hdc,
            &mut lw,
            &mut lrc,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );

        if let Some(o) = old2 {
            SelectObject(hdc, o);
            let _ = DeleteObject(bf);
        }
        let _ = EndPaint(hwnd, &ps);
    }
}

/// 勾选框透明底，避免在暖白底上出现灰块。
unsafe fn ctl_color_btn(hdc: HDC) -> LRESULT {
    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, COLORREF(COL_TEXT));
    LRESULT(GetStockObject(NULL_BRUSH).0 as isize)
}

unsafe extern "system" fn wnd_proc(
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
        WM_ERASEBKGND => LRESULT(1),
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => ctl_color_btn(HDC(wparam.0 as *mut _)),
        WM_LBUTTONDOWN => {
            let cy = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let cx = (lparam.0 & 0xFFFF) as i16 as i32;
            let btn = apply_btn_rect();
            if cx >= btn.left && cx < btn.right && cy >= btn.top && cy < btn.bottom {
                on_apply(hwnd);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as u16;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u16;
            // 勾选变化时只刷新观感，不立刻启停（需点应用）
            if id == IDC_CHECK && (code == BN_CLICKED as u16 || code == 0) {
                let _ = InvalidateRect(hwnd, None, true);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 == VK_ESCAPE.0 as usize {
                let _ = DestroyWindow(hwnd);
            }
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
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
