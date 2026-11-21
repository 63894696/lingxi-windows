# -*- coding: utf-8 -*-
"""灵犀·拼音 — 诊断版
带详细日志，定位候选词选择问题
"""
import tkinter as tk
import threading
import time
import sys
import os
import ctypes
import queue
from ctypes import wintypes

# pythonw 无控制台时 sys.stdout/stderr 为 None,print 会闪退。
# 重定向到 devnull 并持有引用防 GC 关闭(与语音输入法 lingxi_app.py 同款坑)。
_DEVNULL_HOLDS = []
if sys.stdout is None:
    _dn = open(os.devnull, "w", encoding="utf-8", errors="replace")
    _DEVNULL_HOLDS.append(_dn)
    sys.stdout = _dn
if sys.stderr is None:
    _dn2 = open(os.devnull, "w", encoding="utf-8", errors="replace")
    _DEVNULL_HOLDS.append(_dn2)
    sys.stderr = _dn2

# pythonw 下未捕获异常静默丢失 → 全局钩子落盘
def _ime_excepthook(exc_type, exc_value, exc_tb):
    import traceback
    try:
        with open(os.path.join(os.path.dirname(os.path.abspath(__file__)),
                               "pinyin_crash.log"), "a", encoding="utf-8") as f:
            f.write(f"\n=== {time.strftime('%Y-%m-%d %H:%M:%S')} CRASH ===\n")
            traceback.print_exception(exc_type, exc_value, exc_tb, file=f)
    except Exception:
        pass
    sys.__excepthook__(exc_type, exc_value, exc_tb)
sys.excepthook = _ime_excepthook

# 加两级:lingxi_ime(供 from backend.xxx) 和 voice_input(供 from lingxi_ime.backend.xxx)
_here = os.path.dirname(os.path.abspath(__file__))          # frontend/pinyin
_lingxi_ime = os.path.dirname(os.path.dirname(_here))       # lingxi_ime
_voice_input = os.path.dirname(_lingxi_ime)                 # voice_input(lingxi_ime 的父目录)
sys.path.insert(0, _lingxi_ime)
sys.path.insert(0, _voice_input)

from backend.engine import get_engine
from backend.context import get_context

WH_KEYBOARD_LL = 13
WM_KEYDOWN = 0x0100
WM_SYSKEYDOWN = 0x0104
VK_BACK = 0x08
VK_RETURN = 0x0D
VK_ESCAPE = 0x1B
VK_SPACE = 0x20
VK_0 = 0x30
VK_9 = 0x39
VK_RCONTROL = 0xA3

LOG_FILE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "pinyin_diag.log")


def log(msg):
    try:
        with open(LOG_FILE, "a", encoding="utf-8") as f:
            f.write(f"{time.strftime('%H:%M:%S')} {msg}\n")
    except Exception:
        pass


class KBDLLHOOKSTRUCT(ctypes.Structure):
    _fields_ = [
        ("vkCode", wintypes.DWORD),
        ("scanCode", wintypes.DWORD),
        ("flags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong)),
    ]


class GUITHREADINFO_PROBE(ctypes.Structure):
    """GetGUIThreadInfo 用 — Bug1 遥测:跨进程读目标对话框的焦点控件。"""
    _fields_ = [
        ("cbSize", ctypes.c_ulong), ("flags", ctypes.c_ulong),
        ("hwndActive", ctypes.c_void_p), ("hwndFocus", ctypes.c_void_p),
        ("hwndCapture", ctypes.c_void_p), ("hwndMenuOwner", ctypes.c_void_p),
        ("hwndMoveSize", ctypes.c_void_p), ("hwndCaret", ctypes.c_void_p),
        ("rcCaret", ctypes.c_long * 4),
    ]


class PinyinIME:
    def __init__(self):
        self.engine = get_engine()
        self.context = get_context()
        self._load_ui_config()
        self._root = None
        self._input_str = ""
        self._candidates = []
        self._selected = 0
        self._active = False
        self._composing = False  # 原子布尔：组词中/有候选，钩子线程唯一可读的状态
        self._hook = None
        self._hook_proc = None
        self._candidate_win = None
        self._tray = None
        # 控制栏
        self._bar = None
        self._bar_light = None
        self._bar_punct_btn = None
        self._bar_width_btn = None
        self.full_width = False        # 半角/全角
        self.chinese_punct = True      # 中英标点

        self._drag = None
        # 整句逐字选状态：None=不在逐字选；dict=进行中的逐字选
        # {segments:[音节...], pos:当前选到第几个, picked:[已选字...]}
        self._sentence = None
        self._cmd_mode = False  # 超级命令模式(/ 开头)
        self._last_inject_ts = 0.0   # 上次注入 Ctrl+V 结束的时间戳
        self._key_queue = queue.Queue()  # 钩子线程入队，主线程出队
        self._target_hwnd = 0            # 激活那一刻的前台窗口（上屏目标）

    VERSION = "2026-08-19-handwriting-v13"

    def _super_cmd_enabled(self):
        """超级命令是否开启(ime_config.ENABLE_SUPER_COMMAND,默认关)。"""
        try:
            from lingxi_ime.backend import ime_config as cfg
            return bool(getattr(cfg, "ENABLE_SUPER_COMMAND", False))
        except Exception:
            return False

    def _load_ui_config(self):
        """从 ime_config 加载界面配置(字体/字号/候选数/横竖排/配色)。"""
        try:
            from lingxi_ime.backend import ime_config as cfg
            self._font_family = cfg.UI_FONT_FAMILY
            self._font_size = cfg.UI_FONT_SIZE
            self._input_font_size = cfg.UI_INPUT_FONT_SIZE
            self._cand_per_page = cfg.UI_CANDIDATES_PER_PAGE
            self._layout = cfg.UI_LAYOUT
            self._ui_bg = cfg.UI_BG
            self._ui_bg_input = cfg.UI_BG_INPUT
            self._ui_fg_cand = cfg.UI_FG_CAND
            self._ui_fg_sel = cfg.UI_FG_SELECTED
            self._ui_fg_input = cfg.UI_FG_INPUT
        except Exception:
            self._font_family = "Microsoft YaHei UI"
            self._font_size = 12
            self._input_font_size = 12
            self._cand_per_page = 9
            self._layout = "horizontal"
            self._ui_bg = "#ffffff"
            self._ui_bg_input = "#e8f4fd"
            self._ui_fg_cand = "#5b6b7c"
            self._ui_fg_sel = "#c47f2a"
            self._ui_fg_input = "#c47f2a"

    # 单实例(同语音 lingxi_app 机制):命名互斥体,重复启动只退出不抢钩子/托盘。
    _MUTEX_NAME = "LingxiPinyin_SingleInstance_Mutex"

    def _acquire_single_instance(self):
        """已运行则返回 False(本进程应退出)。拼音控制栏常驻,无需弹窗通知。
        例外:「重启接力」进程(LINGXI_RESTARTER=1)遇旧实例退出中互斥体未释放 →
        轮询等待其释放(最多 ~3s)再接管,而不是误判重复启动自杀。"""
        k32 = ctypes.windll.kernel32
        ERROR_ALREADY_EXISTS = 183
        is_restarter = os.environ.get("LINGXI_RESTARTER") == "1"
        self._mutex = k32.CreateMutexW(None, False, self._MUTEX_NAME)
        already = (k32.GetLastError() == ERROR_ALREADY_EXISTS)
        if already and is_restarter:
            for _ in range(30):  # 30 * 100ms = 3s
                time.sleep(0.1)
                self._mutex = k32.CreateMutexW(None, False, self._MUTEX_NAME)
                if k32.GetLastError() != ERROR_ALREADY_EXISTS:
                    already = False
                    break
        return not already

    def start(self):
        log(f"=== start VERSION={self.VERSION} ===")
        if not self._acquire_single_instance():
            log("single instance: already running, exit")
            return
        self._root = tk.Tk()
        self._root.withdraw()
        self._create_tray()
        self._create_bar()
        self._install_hook()
        self._start_toggle_hotkey()
        self._pump_keys()
        # 启动即后台预建内存索引(不等第一次查询,消除"第一次打字等一阵")
        try:
            self.engine._ensure_memory_index()
            log("memory index: prebuild triggered")
        except Exception as e:
            log(f"memory index prebuild failed: {e}")
        log("=== mainloop ===")
        self._root.mainloop()

    def _pump_keys(self):
        """主线程泵：每 15ms 从队列取钩子投递的按键，在主线程安全执行。"""
        try:
            while True:
                vk = self._key_queue.get_nowait()
                self._safe_on_key(vk)
        except queue.Empty:
            pass
        except Exception as e:
            log(f"_pump_keys ERROR: {e}")
        self._root.after(15, self._pump_keys)

    # ---------------- 控制栏 ----------------
    def _create_bar(self):
        """可拖动输入法控制栏：常驻、置顶、不抢焦点。崩溃时随进程消失。"""
        bar = tk.Toplevel(self._root)
        self._bar = bar
        bar.overrideredirect(True)
        bar.attributes("-topmost", True)
        bar.configure(bg="#ffffff", bd=0, highlightthickness=1,
                      highlightbackground="#b3d7f5")
        self._set_no_activate(bar)

        # 状态灯（ Canvas 圆点 ）：灰=未激活 绿=已激活可输入
        light = tk.Canvas(bar, width=18, height=18, bg="#ffffff",
                          highlightthickness=0, cursor="hand2")
        light.pack(side=tk.LEFT, padx=(6, 4), pady=4)
        self._bar_light = light
        light.bind("<Button-1>", lambda e: self._toggle())

        title = tk.Label(bar, text="灵犀·拼音", bg="#ffffff", fg="#5b6b7c",
                         font=("Microsoft YaHei UI", 9), cursor="fleur")
        title.pack(side=tk.LEFT, padx=(0, 6))

        # 中/英标点
        self._bar_punct_btn = tk.Label(
            bar, text="，。", bg="#e8f4fd", fg="#c47f2a",
            font=("Microsoft YaHei UI", 9), padx=6, pady=1, cursor="hand2")
        self._bar_punct_btn.pack(side=tk.LEFT, padx=2)
        self._bar_punct_btn.bind("<Button-1>", lambda e: self._toggle_punct())

        # 半角/全角
        self._bar_width_btn = tk.Label(
            bar, text="半", bg="#e8f4fd", fg="#5b6b7c",
            font=("Microsoft YaHei UI", 9), padx=6, pady=1, cursor="hand2")
        self._bar_width_btn.pack(side=tk.LEFT, padx=2)
        self._bar_width_btn.bind("<Button-1>", lambda e: self._toggle_width())

        # 手写板
        hw = tk.Label(bar, text="写", bg="#e8f4fd", fg="#5b6b7c",
                      font=("Microsoft YaHei UI", 9), padx=6, pady=1, cursor="hand2")
        hw.pack(side=tk.LEFT, padx=2)
        hw.bind("<Button-1>", lambda e: self._toggle_handwriting())

        # AI 转发 PrisirWork
        ai = tk.Label(bar, text="AI", bg="#e8f4fd", fg="#c47f2a",
                      font=("Microsoft YaHei UI", 9, "bold"), padx=6, pady=1, cursor="hand2")
        ai.pack(side=tk.LEFT, padx=2)
        ai.bind("<Button-1>", lambda e: self._forward_to_prisirwork())

        # 拖动：按住标题或灯空白处拖动
        for w in (bar, title):
            w.bind("<ButtonPress-1>", self._bar_drag_start)
            w.bind("<B1-Motion>", self._bar_drag_move)

        # 默认放右上角
        bar.update_idletasks()
        sw = bar.winfo_screenwidth()
        bar.geometry(f"+{sw - 220}+40")
        bar.deiconify()
        self._refresh_bar()
        log("control bar created")

    def _bar_drag_start(self, e):
        self._drag = (e.x_root - self._bar.winfo_x(),
                      e.y_root - self._bar.winfo_y())

    def _bar_drag_move(self, e):
        if not self._drag:
            return
        dx, dy = self._drag
        self._bar.geometry(f"+{e.x_root - dx}+{e.y_root - dy}")

    def _toggle_punct(self):
        self.chinese_punct = not self.chinese_punct
        self._bar_punct_btn.config(
            text="，。" if self.chinese_punct else ",.",
            fg="#c47f2a" if self.chinese_punct else "#5b6b7c")
        log(f"punct: chinese={self.chinese_punct}")

    def _toggle_width(self):
        self.full_width = not self.full_width
        self._bar_width_btn.config(
            text="全" if self.full_width else "半",
            fg="#c47f2a" if self.full_width else "#5b6b7c")
        log(f"width: full={self.full_width}")

    # ---------------- PrisirWork 转发钩子 ----------------
    _PRISIRWORK_HOST = "127.0.0.1"
    _PRISIRWORK_PORT = 51234

    def _forward_to_prisirwork(self):
        """把当前内容转发给 PrisirWork 处理,结果回流上屏。
        内容来源:优先当前拼音输入串,否则取剪贴板(用户选中文本)。
        异步发送,不阻塞输入。"""
        text = self._input_str
        intent = "polish"
        if not text:
            try:
                import pyperclip
                clip = pyperclip.paste()
                if clip and clip.strip():
                    text = clip.strip()
                    intent = "raw"
            except Exception:
                pass
        if not text:
            log("prisirwork: nothing to forward")
            return
        log(f"prisirwork: forward intent={intent} text={text!r}")
        threading.Thread(target=self._prisirwork_rpc, args=(text, intent), daemon=True).start()

    def _prisirwork_rpc(self, text, intent):
        """socket 客户端:发请求给 PrisirWork,收结果,主线程上屏。"""
        import json, socket
        try:
            ctx = {"app": "lingxi_pinyin", "hwnd": self._target_hwnd}
            req = {"type": "ime_forward", "text": text,
                   "intent": intent, "context": ctx}
            with socket.create_connection((self._PRISIRWORK_HOST, self._PRISIRWORK_PORT), timeout=10) as s:
                s.sendall((json.dumps(req, ensure_ascii=False) + "\n").encode("utf-8"))
                f = s.makefile("r", encoding="utf-8")
                line = f.readline()
            resp = json.loads(line) if line else {}
            if resp.get("ok") and resp.get("text"):
                out = resp["text"]
                log(f"prisirwork: result {out!r}")
                # 回到主线程上屏
                self._root.after(0, lambda: self._prisirwork_commit(out))
            else:
                log("prisirwork: empty/failed result")
        except Exception as e:
            log(f"prisirwork rpc failed: {e}")

    def _prisirwork_commit(self, text):
        """PrisirWork 结果回流:清掉当前输入(若转发的是输入串),上屏处理结果。"""
        if self._input_str:
            self._input_str = ""
            self._candidates = []
            self._set_composing(False)
            self._hide_candidates()
        self._send_text(text)

    # ---------------- 手写板（鼠标写字，Windows Ink 识别）----------------
    def _toggle_handwriting(self):
        """打开/关闭手写板。"""
        if getattr(self, "_hw_win", None) and self._hw_win.winfo_exists():
            self._close_handwriting()
        else:
            self._open_handwriting()

    def _open_handwriting(self):
        """打开手写板：InkCollector 绑窗口收笔迹，松开鼠标自动识别给候选。"""
        try:
            import win32com.client
            import pythoncom
        except ImportError:
            log("handwriting: pywin32 不可用")
            return
        win = tk.Toplevel(self._root)
        self._hw_win = win
        win.title("手写")
        win.attributes("-topmost", True)
        win.configure(bg="#ffffff")
        # 关键:手写板不抢焦点,目标窗口(记事本)保持前台,候选才能上屏到正确位置
        self._set_no_activate(win)
        # 候选区（固定高度，避免出候选时窗口变高）
        self._hw_cand = tk.Frame(win, bg="#ffffff", height=44)
        self._hw_cand.pack(fill=tk.X, padx=4, pady=(4, 0))
        self._hw_cand.pack_propagate(False)
        # 写字区（InkCollector 直接收系统笔迹，背景色由 Ink 绘制）
        self._hw_area = tk.Frame(win, bg="#ffffff", width=360, height=240)
        self._hw_area.pack(padx=4, pady=4)
        self._hw_area.pack_propagate(False)
        # 按钮区：识别(手动) / 清除 / 关闭
        btns = tk.Frame(win, bg="#ffffff")
        btns.pack(fill=tk.X, padx=4, pady=(0, 4))
        tk.Button(btns, text="识别", command=self._hw_recognize,
                  bg="#128a4a", fg="#ffffff", relief=tk.FLAT,
                  font=("Microsoft YaHei UI", 9, "bold"), padx=12).pack(side=tk.LEFT, padx=2)
        tk.Button(btns, text="清除", command=self._hw_clear,
                  bg="#e8f4fd", fg="#5b6b7c", relief=tk.FLAT).pack(side=tk.LEFT, padx=2)
        tk.Button(btns, text="关闭", command=self._close_handwriting,
                  bg="#e8f4fd", fg="#5b6b7c", relief=tk.FLAT).pack(side=tk.RIGHT, padx=2)

        win.update_idletasks()
        hwnd = self._hw_area.winfo_id()

        # 记录打开手写板时的前台窗口作为上屏目标(避免手写板干扰上屏目标判断)
        try:
            fg = ctypes.windll.user32.GetForegroundWindow()
            if not self._is_own_window(fg):
                self._target_hwnd = fg
                log(f"handwriting: target hwnd={fg}")
        except Exception:
            pass

        # InkCollector 绑写字区窗口
        self._hw_coll = win32com.client.Dispatch("msinkaut.InkCollector.1")
        self._hw_coll.hWnd = hwnd
        self._hw_coll.Enabled = True
        # 笔迹样式：细黑线
        try:
            da = self._hw_coll.DefaultDrawingAttributes
            da.Color = 0x202020
            da.Width = 180  # ink 单位
        except Exception:
            pass
        # 不自动识别：写完整个字后点"识别"按钮(或按空格)手动触发
        # 空格也触发识别(写字区可能无焦点,绑在窗口上)
        win.bind("<space>", lambda e: self._hw_recognize())
        win.protocol("WM_DELETE_WINDOW", self._close_handwriting)
        # 固定窗口大小,禁止书写中因候选/布局改变
        win.resizable(False, False)
        log("handwriting: opened")

    def _hw_clear(self):
        """清除笔迹和候选。"""
        try:
            coll = getattr(self, "_hw_coll", None)
            if coll:
                coll.Ink.DeleteStrokes()
        except Exception as e:
            log(f"hw_clear failed: {e}")
        try:
            for w in self._hw_cand.winfo_children():
                w.destroy()
        except Exception:
            pass

    def _hw_recognize(self):
        """识别当前笔迹，显示候选汉字。"""
        try:
            coll = getattr(self, "_hw_coll", None)
            if not coll or coll.Ink.Strokes.Count == 0:
                return
            import win32com.client
            ctx = win32com.client.Dispatch("msinkaut.InkRecognizerContext")
            ctx.Strokes = coll.Ink.Strokes
            res = ctx.Recognize()
            result = res[0] if isinstance(res, tuple) else res
            # 取前 8 候选
            cands = []
            try:
                alts = result.AlternatesFromSelection(0, len(result.TopString), 8)
                for i in range(alts.Count):
                    cands.append(alts.Item(i).String)
            except Exception:
                cands = [result.TopString]
            self._hw_show_cands(cands)
        except Exception as e:
            log(f"handwriting recognize failed: {e}")

    def _hw_show_cands(self, cands):
        """在候选区显示候选字，点击上屏并清除笔迹。"""
        for w in self._hw_cand.winfo_children():
            w.destroy()
        for ch in cands:
            lbl = tk.Label(self._hw_cand, text=ch, bg="#e8f4fd", fg="#c47f2a",
                           font=("Microsoft YaHei UI", 16), padx=10, pady=4,
                           cursor="hand2")
            lbl.pack(side=tk.LEFT, padx=2)
            lbl.bind("<Button-1>", lambda e, c=ch: self._hw_commit(c))

    def _hw_commit(self, ch):
        """候选上屏，清除笔迹。"""
        log(f"handwriting: commit {ch!r}")
        self._send_text(ch)
        self._hw_clear()

    def _close_handwriting(self):
        """关闭手写板，释放 InkCollector。"""
        try:
            if getattr(self, "_hw_coll", None):
                self._hw_coll.Enabled = False
                self._hw_coll = None
        except Exception:
            pass
        try:
            if getattr(self, "_hw_win", None) and self._hw_win.winfo_exists():
                self._hw_win.destroy()
        except Exception:
            pass
        self._hw_win = None
        log("handwriting: closed")

    def _refresh_bar(self):
        """刷新状态灯：灰=未激活 绿=已激活可输入 亮绿闪=组词中"""
        if not self._bar_light:
            return
        c = self._bar_light
        c.delete("all")
        if not self._active:
            fill = "#9aa3b2"          # 灰：未激活
        elif self._composing:
            fill = "#2fb36a"          # 亮绿：组词中
        else:
            fill = "#128a4a"          # 绿：已激活可输入
        c.create_oval(3, 3, 15, 15, fill=fill, outline="")


    def _create_tray(self):
        try:
            import pystray
            from PIL import Image, ImageDraw
            ico_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "pinyin.ico")
            if os.path.exists(ico_path):
                img = Image.open(ico_path)
            else:
                img = Image.new("RGB", (64, 64), "#ffffff")
                d = ImageDraw.Draw(img)
                d.ellipse([8, 8, 56, 56], fill="#128a4a")
                d.text((22, 18), "拼", fill="#ffffff")
            def _menu_cb(name, fn):
                def cb(icon, item):
                    log(f"tray menu clicked: {name}")
                    try:
                        self._root.after(0, fn)
                    except Exception as e:
                        log(f"tray menu cb error {name}: {e}")
                return cb
            menu = pystray.Menu(
                # pystray 菜单回调在 pystray 线程跑,tkinter 窗口必须主线程创建,
                # 故用 after(0, ...) 投递到主线程,否则点击静默无反应
                pystray.MenuItem("设置", _menu_cb("设置", self._open_settings)),
                pystray.MenuItem("词库管理", _menu_cb("词库管理", self._open_dict_manager)),
                pystray.MenuItem("重启", _menu_cb("重启", self._restart)),
                pystray.MenuItem("退出", lambda icon, item: self._root.after(0, self._quit))
            )
            self._tray = pystray.Icon("lingxi_pinyin", img, "灵犀·拼音", menu)
            threading.Thread(target=self._tray.run, daemon=True).start()
            log("tray created")
        except Exception as e:
            log(f"tray failed: {e}")

    # ---------------- 设置面板 ----------------
    def _open_settings(self):
        """界面设置面板:字号/候选数/横竖排/模糊音,保存即生效(写配置+热更新)。"""
        try:
            self._open_settings_impl()
        except Exception as e:
            log(f"_open_settings ERROR: {e}")
            import traceback
            log(traceback.format_exc())

    def _open_settings_impl(self):
        from lingxi_ime.backend import ime_config as cfg
        from lingxi_ime.backend import hotkey_field as hf
        log("_open_settings: creating window")
        win = tk.Toplevel(self._root)
        self._set_win = win
        win.title("灵犀·拼音 — 设置")
        win.geometry("460x520")
        win.configure(bg="#ffffff")

        row = 0
        pad = dict(padx=16, pady=8)
        def label(text):
            return tk.Label(win, text=text, bg="#ffffff", fg="#5b6b7c",
                            font=("Microsoft YaHei UI", 10))

        # 候选字号
        label("候选字号:").grid(row=row, column=0, sticky="w", **pad)
        self._set_fontsize = tk.Entry(win, bg="#e8f4fd", fg="#1f2937",
                                       font=("Microsoft YaHei UI", 10), width=8)
        self._set_fontsize.grid(row=row, column=1, sticky="w", **pad)
        self._set_fontsize.insert(0, str(self._font_size))
        row += 1

        # 每页候选数
        label("每页候选数:").grid(row=row, column=0, sticky="w", **pad)
        self._set_perpage = tk.Entry(win, bg="#e8f4fd", fg="#1f2937",
                                      font=("Microsoft YaHei UI", 10), width=8)
        self._set_perpage.grid(row=row, column=1, sticky="w", **pad)
        self._set_perpage.insert(0, str(self._cand_per_page))
        row += 1

        # 排列方式
        label("排列方式:").grid(row=row, column=0, sticky="w", **pad)
        self._set_layout = tk.StringVar(value=self._layout)
        lf = tk.Frame(win, bg="#ffffff")
        lf.grid(row=row, column=1, sticky="w", **pad)
        tk.Radiobutton(lf, text="横排", variable=self._set_layout, value="horizontal",
                       bg="#ffffff", fg="#1f2937", selectcolor="#e8f4fd",
                       font=("Microsoft YaHei UI", 10)).pack(side=tk.LEFT)
        tk.Radiobutton(lf, text="竖排", variable=self._set_layout, value="vertical",
                       bg="#ffffff", fg="#1f2937", selectcolor="#e8f4fd",
                       font=("Microsoft YaHei UI", 10)).pack(side=tk.LEFT, padx=8)
        row += 1

        # 模糊音（参考影子输入法:声母 + 前后鼻音 + 韵母）
        label("模糊音:").grid(row=row, column=0, sticky="nw", **pad)
        cur = set(getattr(cfg, "FUZZY_RULES", frozenset()))
        fuzzy_defs = [
            ("z_zh", "z=zh"), ("c_ch", "c=ch"), ("s_sh", "s=sh"),
            ("an_ang", "an=ang"), ("en_eng", "en=eng"), ("in_ing", "in=ing"),
            ("ian_iang", "ian=iang"), ("uan_uang", "uan=uang"), ("ai_an", "ai=an"),
            ("un_ong", "un=ong"),
        ]
        self._set_fuzzy = {r: tk.BooleanVar(value=(r in cur)) for r, _ in fuzzy_defs}
        ff = tk.Frame(win, bg="#ffffff")
        ff.grid(row=row, column=1, sticky="w", **pad)
        for idx, (r, name) in enumerate(fuzzy_defs):
            tk.Checkbutton(ff, text=name, variable=self._set_fuzzy[r],
                           bg="#ffffff", fg="#1f2937", selectcolor="#e8f4fd",
                           font=("Microsoft YaHei UI", 10)).grid(
                               row=idx // 3, column=idx % 3, sticky="w", padx=(0, 10))
        row += 1

        # 激活热键(2026-08-21 复制自语音 lingxi_app 的「热键设置」,共用 hotkey_field 组件)
        label("激活热键:").grid(row=row, column=0, sticky="nw", **pad)
        cur_vk = getattr(cfg, "PINYIN_TRIGGER_KEY", 0xA3)
        self._pending_trigger = cur_vk  # 捕获成功先暂存,点「保存并生效」才落盘+生效
        self._hotkey_field = hf.HotkeyField(
            win, current_vk=cur_vk, on_change=self._on_trigger_captured)
        self._hotkey_field.grid(row=row, column=1, sticky="w", **pad)
        row += 1

        # 保存按钮 + 状态
        tk.Button(win, text="保存并生效", command=self._settings_save,
                  bg="#128a4a", fg="#ffffff", relief=tk.FLAT,
                  font=("Microsoft YaHei UI", 11, "bold"), padx=20, pady=4,
                  cursor="hand2").grid(row=row, column=0, columnspan=2, pady=16)
        row += 1
        self._set_status = tk.Label(win, text="", bg="#ffffff", fg="#9aa3b2",
                                     font=("Microsoft YaHei UI", 9))
        self._set_status.grid(row=row, column=0, columnspan=2)
        log("settings opened")

    def _on_trigger_captured(self, vk):
        """热键采集回调:冲突检测(与五笔/语音错开)通过后暂存,等保存统一下发。
        返回 None=接受;返回 str=拒绝理由(组件红字提示,不落盘)。"""
        from lingxi_ime.backend import ime_config as cfg
        from lingxi_ime.backend import hotkey_field as hf
        wubi_vk = getattr(cfg, "WUBI_TRIGGER_KEY", 0xA1)
        if vk == wubi_vk:
            return f"{hf.vk_name(vk)} 已被五笔占用,请换一键"
        try:
            from lingxi_config import TRIGGER_KEY as voice_vk
            if vk == voice_vk:
                return f"{hf.vk_name(vk)} 已被语音占用,请换一键"
        except Exception:
            pass
        self._pending_trigger = vk
        return None

    def _apply_trigger_key(self, vk):
        """立即生效:改轮询读的 override 变量。"""
        self._trigger_key_override = vk

    def _settings_save(self):
        """保存设置:写回 ime_config.py + 热更新(重建候选窗)。"""
        # 读值
        try:
            fontsize = int(self._set_fontsize.get().strip())
            perpage = int(self._set_perpage.get().strip())
            if not (6 <= fontsize <= 40 and 1 <= perpage <= 20):
                raise ValueError
        except ValueError:
            self._set_status.config(text="字号 6-40,候选数 1-20")
            return
        layout = self._set_layout.get()
        fuzzy = frozenset(r for r, v in self._set_fuzzy.items() if v.get())

        # 写回 ime_config.py(替换对应行)
        ok = self._write_ui_config(fontsize, perpage, layout, fuzzy)
        if not ok:
            self._set_status.config(text="写入配置失败")
            return

        # 激活热键:若本次有改动,落盘 + 立即生效(改轮询 override)
        trigger = getattr(self, "_pending_trigger", None)
        cur_vk = getattr(cfg, "PINYIN_TRIGGER_KEY", 0xA3)
        if trigger and trigger != cur_vk:
            self._write_trigger_key(trigger)
            self._apply_trigger_key(trigger)
            log(f"pinyin trigger key -> {trigger:#x}")

        # 热更新:重新加载配置 + 重建候选窗
        self._font_size = fontsize
        self._input_font_size = fontsize
        self._cand_per_page = perpage
        self._layout = layout
        if self._candidate_win is not None:
            try:
                self._candidate_win.destroy()
            except Exception:
                pass
            self._candidate_win = None
        log(f"settings saved: fontsize={fontsize} perpage={perpage} layout={layout} fuzzy={fuzzy}")
        self._set_status.config(text="已保存并生效")

    def _write_ui_config(self, fontsize, perpage, layout, fuzzy):
        """把设置写回 ime_config.py(精准替换对应配置行)。"""
        try:
            path = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(
                os.path.abspath(__file__)))), "backend", "ime_config.py")
            with open(path, "r", encoding="utf-8") as f:
                text = f.read()
            import re
            fuzzy_str = "frozenset({" + ", ".join(f'"{r}"' for r in sorted(fuzzy)) + "})" if fuzzy else "frozenset()"
            text = re.sub(r"FUZZY_RULES = .*", f"FUZZY_RULES = {fuzzy_str}", text)
            text = re.sub(r"UI_FONT_SIZE = \d+", f"UI_FONT_SIZE = {fontsize}", text)
            text = re.sub(r"UI_INPUT_FONT_SIZE = \d+", f"UI_INPUT_FONT_SIZE = {fontsize}", text)
            text = re.sub(r"UI_CANDIDATES_PER_PAGE = \d+", f"UI_CANDIDATES_PER_PAGE = {perpage}", text)
            text = re.sub(r'UI_LAYOUT = "\w+"', f'UI_LAYOUT = "{layout}"', text)
            with open(path, "w", encoding="utf-8") as f:
                f.write(text)
            return True
        except Exception as e:
            log(f"write ui config failed: {e}")
            return False

    def _write_trigger_key(self, vk):
        """把拼音激活键写回 ime_config.py 的 PINYIN_TRIGGER_KEY(同 _write_ui_config 精准替换)。"""
        try:
            path = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(
                os.path.abspath(__file__)))), "backend", "ime_config.py")
            with open(path, "r", encoding="utf-8") as f:
                text = f.read()
            import re
            text = re.sub(r"PINYIN_TRIGGER_KEY = 0x[0-9A-Fa-f]+",
                          f"PINYIN_TRIGGER_KEY = 0x{vk:02X}", text)
            with open(path, "w", encoding="utf-8") as f:
                f.write(text)
            return True
        except Exception as e:
            log(f"write pinyin trigger failed: {e}")
            return False

    def _open_dict_manager(self):
        """术语库编辑器:增删改查 + 保存即生效(写库 + 增量更新内存索引)。"""
        try:
            self._open_dict_manager_impl()
        except Exception as e:
            log(f"_open_dict_manager ERROR: {e}")
            import traceback
            log(traceback.format_exc())

    def _open_dict_manager_impl(self):
        from lingxi_ime.backend.database import get_db
        log("_open_dict_manager: creating window")
        db = get_db()
        win = tk.Toplevel(self._root)
        self._dict_win = win
        win.title("灵犀·拼音 — 术语库编辑器")
        win.geometry("720x520")
        win.configure(bg="#ffffff")
        self._dict_db = db
        self._dict_offset = 0
        self._dict_page_size = 20
        self._dict_selected = None  # (key, value)

        # ---- 顶部:搜索 + 新增 ----
        top = tk.Frame(win, bg="#ffffff")
        top.pack(fill=tk.X, padx=12, pady=(12, 4))

        tk.Label(top, text="搜索:", bg="#ffffff", fg="#5b6b7c",
                 font=("Microsoft YaHei UI", 10)).pack(side=tk.LEFT)
        self._dict_search = tk.Entry(top, bg="#e8f4fd", fg="#1f2937",
                                      font=("Microsoft YaHei UI", 10), width=22)
        self._dict_search.pack(side=tk.LEFT, padx=(6, 6))
        self._dict_search.bind("<KeyRelease>", lambda e: self._dict_refresh(reset=True))
        tk.Button(top, text="搜索", command=lambda: self._dict_refresh(reset=True),
                  bg="#e8f4fd", fg="#5b6b7c", relief=tk.FLAT).pack(side=tk.LEFT)

        # ---- 新增/编辑区 ----
        edit = tk.Frame(win, bg="#ffffff")
        edit.pack(fill=tk.X, padx=12, pady=4)
        tk.Label(edit, text="拼音:", bg="#ffffff", fg="#5b6b7c",
                 font=("Microsoft YaHei UI", 10)).grid(row=0, column=0)
        self._dict_pinyin = tk.Entry(edit, bg="#e8f4fd", fg="#1f2937",
                                      font=("Microsoft YaHei UI", 10), width=16)
        self._dict_pinyin.grid(row=0, column=1, padx=(4, 10))
        tk.Label(edit, text="词语:", bg="#ffffff", fg="#5b6b7c",
                 font=("Microsoft YaHei UI", 10)).grid(row=0, column=2)
        self._dict_word = tk.Entry(edit, bg="#e8f4fd", fg="#1f2937",
                                    font=("Microsoft YaHei UI", 10), width=14)
        self._dict_word.grid(row=0, column=3, padx=(4, 10))
        tk.Label(edit, text="词频:", bg="#ffffff", fg="#5b6b7c",
                 font=("Microsoft YaHei UI", 10)).grid(row=0, column=4)
        self._dict_weight = tk.Entry(edit, bg="#e8f4fd", fg="#1f2937",
                                      font=("Microsoft YaHei UI", 10), width=8)
        self._dict_weight.grid(row=0, column=5, padx=(4, 10))
        self._dict_weight.insert(0, "50000")
        tk.Button(edit, text="保存", command=self._dict_save,
                  bg="#128a4a", fg="#ffffff", relief=tk.FLAT,
                  font=("Microsoft YaHei UI", 10, "bold"), padx=14).grid(row=0, column=6, padx=4)
        tk.Button(edit, text="删除", command=self._dict_delete,
                  bg="#c13a3a", fg="#ffffff", relief=tk.FLAT,
                  font=("Microsoft YaHei UI", 10), padx=14).grid(row=0, column=7, padx=4)

        # ---- 列表(Treeview) ----
        import tkinter.ttk as ttk
        style = ttk.Style(win)
        style.theme_use("clam")
        style.configure("Dict.Treeview", background="#e8f4fd", foreground="#1f2937",
                        fieldbackground="#e8f4fd", rowheight=24,
                        font=("Microsoft YaHei UI", 10))
        style.configure("Dict.Treeview.Heading", background="#ffffff", foreground="#c47f2a",
                        font=("Microsoft YaHei UI", 10, "bold"))
        style.map("Dict.Treeview", background=[("selected", "#cfe8fb")])

        cols = ("pinyin", "word", "weight")
        self._dict_tree = ttk.Treeview(win, columns=cols, show="headings",
                                       style="Dict.Treeview", selectmode="browse")
        self._dict_tree.heading("pinyin", text="拼音")
        self._dict_tree.heading("word", text="词语")
        self._dict_tree.heading("weight", text="词频")
        self._dict_tree.column("pinyin", width=200)
        self._dict_tree.column("word", width=300)
        self._dict_tree.column("weight", width=120)
        sb = ttk.Scrollbar(win, orient="vertical", command=self._dict_tree.yview)
        self._dict_tree.configure(yscrollcommand=sb.set)
        self._dict_tree.pack(side=tk.LEFT, fill=tk.BOTH, expand=True, padx=(12, 0), pady=4)
        sb.pack(side=tk.LEFT, fill=tk.Y, pady=4, padx=(0, 12))
        self._dict_tree.bind("<<TreeviewSelect>>", self._dict_on_select)

        # ---- 底部:分页 + 黑白名单 + 状态 ----
        bottom = tk.Frame(win, bg="#ffffff")
        bottom.pack(fill=tk.X, padx=12, pady=(4, 12))
        tk.Button(bottom, text="< 上一页", command=lambda: self._dict_page(-1),
                  bg="#e8f4fd", fg="#5b6b7c", relief=tk.FLAT).pack(side=tk.LEFT)
        tk.Button(bottom, text="下一页 >", command=lambda: self._dict_page(1),
                  bg="#e8f4fd", fg="#5b6b7c", relief=tk.FLAT).pack(side=tk.LEFT, padx=6)

        # 分隔线
        tk.Frame(bottom, width=1, bg="#cccccc").pack(side=tk.LEFT, fill=tk.Y, padx=8, pady=4)

        # 黑白名单按钮
        tk.Button(bottom, text="查看黑名单", command=self._dict_view_blacklist,
                  bg="#fdecea", fg="#c13a3a", relief=tk.FLAT,
                  font=("Microsoft YaHei UI", 9), padx=8).pack(side=tk.LEFT, padx=2)
        tk.Button(bottom, text="查看白名单", command=self._dict_view_whitelist,
                  bg="#e8f4ea", fg="#128a4a", relief=tk.FLAT,
                  font=("Microsoft YaHei UI", 9), padx=8).pack(side=tk.LEFT, padx=2)
        tk.Button(bottom, text="自动别名", command=self._dict_view_auto_aliases,
                  bg="#e8f4fd", fg="#c47f2a", relief=tk.FLAT,
                  font=("Microsoft YaHei UI", 9), padx=8).pack(side=tk.LEFT, padx=2)

        self._dict_status = tk.Label(bottom, text="", bg="#ffffff", fg="#9aa3b2",
                                      font=("Microsoft YaHei UI", 9))
        self._dict_status.pack(side=tk.LEFT, padx=12)

        self._dict_refresh(reset=True)
        log("dict editor opened")

    # ---- 编辑器逻辑 ----
    def _dict_refresh(self, reset=False):
        """刷新词条列表。"""
        if reset:
            self._dict_offset = 0
        kw = self._dict_search.get().strip()
        db = self._dict_db
        rows = db.dict_list(kw, limit=self._dict_page_size, offset=self._dict_offset)
        total = db.dict_count(kw)
        for item in self._dict_tree.get_children():
            self._dict_tree.delete(item)
        for key, value, weight in rows:
            self._dict_tree.insert("", tk.END, values=(key, value, weight))
        page = self._dict_offset // self._dict_page_size + 1
        pages = max(1, (total + self._dict_page_size - 1) // self._dict_page_size)
        self._dict_status.config(
            text=f"第 {page}/{pages} 页 · 共 {total} 条 · 保存即生效")

    def _dict_page(self, delta):
        kw = self._dict_search.get().strip()
        total = self._dict_db.dict_count(kw)
        new_off = self._dict_offset + delta * self._dict_page_size
        if 0 <= new_off < total:
            self._dict_offset = new_off
            self._dict_refresh()

    def _dict_on_select(self, event):
        sel = self._dict_tree.selection()
        if not sel:
            return
        vals = self._dict_tree.item(sel[0], "values")
        if not vals:
            return
        self._dict_selected = (vals[0], vals[1])
        self._dict_pinyin.delete(0, tk.END); self._dict_pinyin.insert(0, vals[0])
        self._dict_word.delete(0, tk.END); self._dict_word.insert(0, vals[1])
        self._dict_weight.delete(0, tk.END); self._dict_weight.insert(0, vals[2])

    def _dict_save(self):
        """保存(新增/更新):写库 + 增量更新内存索引。"""
        pinyin = self._dict_pinyin.get().strip().lower()
        word = self._dict_word.get().strip()
        weight_s = self._dict_weight.get().strip()
        if not pinyin or not word:
            self._dict_status.config(text="拼音和词语不能为空")
            return
        try:
            weight = int(weight_s) if weight_s else 50000
        except ValueError:
            self._dict_status.config(text="词频必须是数字")
            return
        # 自动提频:提到同 key 最高词频+1000,保证加/改的词排第一,不被原有高词频词组压住
        try:
            top = self._dict_db.dict_max_weight(pinyin)
            if weight < top + 1000:
                weight = top + 1000
        except Exception:
            pass
        try:
            self._dict_db.dict_upsert(pinyin, word, weight)
            # 增量更新内存索引(若已就绪)
            mem = self.engine._mem
            if mem.ready:
                mem.update_weight(pinyin, word, weight)
            log(f"dict saved: {pinyin} -> {word} ({weight})")
            self._dict_status.config(text=f"已保存 {word}(词频{weight})，立即生效")
            self._dict_refresh()
        except Exception as e:
            log(f"dict save failed: {e}")
            self._dict_status.config(text=f"保存失败: {e}")

    def _dict_delete(self):
        """删除选中词条:写库 + 内存索引移除。"""
        pinyin = self._dict_pinyin.get().strip().lower()
        word = self._dict_word.get().strip()
        if not pinyin or not word:
            self._dict_status.config(text="先选中要删除的词条")
            return
        try:
            self._dict_db.dict_delete(pinyin, word)
            mem = self.engine._mem
            if mem.ready:
                mem.remove(pinyin, word)
            log(f"dict deleted: {pinyin} -> {word}")
            self._dict_status.config(text=f"已删除 {word}")
            self._dict_pinyin.delete(0, tk.END)
            self._dict_word.delete(0, tk.END)
            self._dict_refresh()
        except Exception as e:
            log(f"dict delete failed: {e}")
            self._dict_status.config(text=f"删除失败: {e}")

    # ---- 黑白名单查看窗口 ---- 
    def _dict_view_blacklist(self):
        """查看黑名单词条。"""
        self._dict_view_list("黑名单", self._dict_db.get_blacklist(),
                             lambda v: self._dict_db.remove_from_blacklist(v),
                             "已从黑名单移除")

    def _dict_view_whitelist(self):
        """查看白名单词条。"""
        self._dict_view_list("白名单", self._dict_db.get_whitelist(),
                             lambda v: self._dict_db.remove_from_whitelist(v),
                             "已从白名单移除")

    def _dict_view_auto_aliases(self):
        """查看当前选中词条的自动别名建议。"""
        if not self._dict_selected:
            self._dict_status.config(text="先选中一个词条")
            return
        pinyin, word = self._dict_selected
        suggestions = self._dict_db.get_alias_suggestions(word)
        if suggestions:
            self._dict_status.config(text=f"{word} 可添加别名: {', '.join(suggestions)}")
        else:
            self._dict_status.config(text=f"{word} 已无可添加别名")

    def _dict_view_list(self, title: str, rows: List[Tuple[str, str]],
                        remove_cb, remove_msg: str):
        """通用黑白名单查看窗口。"""
        win = tk.Toplevel(self._root)
        win.title(f"灵犀·拼音 — {title}")
        win.geometry("500x400")
        win.configure(bg="#ffffff")

        tk.Label(win, text=f"{title}词条 (共 {len(rows)} 条)", bg="#ffffff",
                 fg="#5b6b7c", font=("Microsoft YaHei UI", 11, "bold")).pack(pady=(12, 6))

        import tkinter.ttk as ttk
        cols = ("word", "reason")
        tree = ttk.Treeview(win, columns=cols, show="headings",
                            style="Dict.Treeview", selectmode="browse")
        tree.heading("word", text="词语")
        tree.heading("reason", text="原因")
        tree.column("word", width=200)
        tree.column("reason", width=260)
        tree.pack(side=tk.LEFT, fill=tk.BOTH, expand=True, padx=(12, 0), pady=4)
        sb = ttk.Scrollbar(win, orient="vertical", command=tree.yview)
        tree.configure(yscrollcommand=sb.set)
        sb.pack(side=tk.LEFT, fill=tk.Y, pady=4, padx=(0, 12))

        for value, reason in rows:
            tree.insert("", tk.END, values=(value, reason or ""))

        btn_frame = tk.Frame(win, bg="#ffffff")
        btn_frame.pack(fill=tk.X, padx=12, pady=8)

        def on_remove():
            sel = tree.selection()
            if not sel:
                return
            vals = tree.item(sel[0], "values")
            if not vals:
                return
            remove_cb(vals[0])
            tree.delete(sel[0])
            self._dict_status.config(text=f"{vals[0]} {remove_msg}")

        tk.Button(btn_frame, text="移除选中", command=on_remove,
                  bg="#c13a3a", fg="#ffffff", relief=tk.FLAT,
                  font=("Microsoft YaHei UI", 10)).pack(side=tk.LEFT, padx=4)
        tk.Button(btn_frame, text="关闭", command=win.destroy,
                  bg="#e8f4fd", fg="#5b6b7c", relief=tk.FLAT,
                  font=("Microsoft YaHei UI", 10)).pack(side=tk.LEFT, padx=4)

    def _quit(self):
        if self._tray:
            self._tray.stop()
        self._root.quit()

    def _restart(self):
        """重启输入法:意外卡死时复位工作状态(2026-08-21)。
        先 spawn 一个新进程(脚本同级 .restarter 标记防递归),再退出本进程。
        单实例互斥体会让新进程在本进程退出前短暂等待后接管钩子/托盘。"""
        try:
            import subprocess
            env = os.environ.copy()
            env["LINGXI_RESTARTER"] = "1"
            subprocess.Popen(
                [sys.executable, os.path.abspath(__file__)],
                cwd=os.path.dirname(os.path.abspath(__file__)),
                env=env, close_fds=True)
            log("restart: spawned new process")
        except Exception as e:
            log(f"restart spawn failed: {e}")
        self._root.after(300, self._quit)  # 稍候再退,让新进程先起

    def _install_hook(self):
        user32 = ctypes.windll.user32
        k32 = ctypes.windll.kernel32
        k32.GetModuleHandleW.restype = wintypes.HMODULE
        user32.SetWindowsHookExW.restype = wintypes.HHOOK
        user32.CallNextHookEx.restype = ctypes.c_long
        user32.CallNextHookEx.argtypes = [wintypes.HHOOK, ctypes.c_int, wintypes.WPARAM, wintypes.LPARAM]

        HOOKPROC = ctypes.WINFUNCTYPE(ctypes.c_long, ctypes.c_int, wintypes.WPARAM, wintypes.LPARAM)
        user32.SetWindowsHookExW.argtypes = [ctypes.c_int, HOOKPROC, wintypes.HMODULE, wintypes.DWORD]

        hmod = k32.GetModuleHandleW(None)

        WM_KEYUP = 0x0101
        WM_SYSKEYUP = 0x0105
        # 物理修饰键状态:只跟踪非注入(真实)按键,注入的 Ctrl+V 不影响它。
        # 用此判断 Ctrl 组合,而非 GetAsyncKeyState(它分不清物理键和注入残留)。
        phys = {"ctrl": False, "alt": False, "win": False}
        self._phys_mod = phys

        def _hook_proc(nCode, wParam, lParam):
            # 键盘钩子回调：必须尽快返回，绝不抛异常、不碰 SQLite/Tk。
            # 只判断“要不要吞这个键”，真正逻辑投递到主线程 via _root.after。
            try:
                if nCode == 0:
                    kb = ctypes.cast(lParam, ctypes.POINTER(KBDLLHOOKSTRUCT)).contents
                    # 跟踪物理 Ctrl/Alt/Win(忽略注入键,LLKHF_INJECTED=0x10)
                    if not (kb.flags & 0x10):
                        if kb.vkCode in (0x11, 0xA2, 0xA3):
                            phys["ctrl"] = wParam in (WM_KEYDOWN, WM_SYSKEYDOWN)
                        elif kb.vkCode in (0x12, 0xA4, 0xA5):
                            phys["alt"] = wParam in (WM_KEYDOWN, WM_SYSKEYDOWN)
                        elif kb.vkCode in (0x5B, 0x5C):
                            phys["win"] = wParam in (WM_KEYDOWN, WM_SYSKEYDOWN)
                    if kb.flags & 0x10:  # synthetic input 放行
                        return user32.CallNextHookEx(self._hook, nCode, wParam, lParam)
                    if getattr(self, "_injecting", False):
                        return user32.CallNextHookEx(self._hook, nCode, wParam, lParam)
                    if wParam in (WM_KEYDOWN, WM_SYSKEYDOWN):
                        if self._active and self._wants_key(kb.vkCode):
                            # 吞掉。钩子里绝不动 Tk：只入队，主线程轮询队列取。
                            self._key_queue.put(kb.vkCode)
                            return 1
            except Exception:
                pass  # 钩子里静默，绝不向外抛
            return user32.CallNextHookEx(self._hook, nCode, wParam, lParam)

        self._hook_proc = HOOKPROC(_hook_proc)
        self._hook = user32.SetWindowsHookExW(WH_KEYBOARD_LL, self._hook_proc, hmod, 0)
        if not self._hook:
            log(f"hook install failed: {k32.GetLastError()}")
            return False
        log("hook installed")
        return True

    def _start_toggle_hotkey(self):
        # 激活键:默认取配置 PINYIN_TRIGGER_KEY,设置面板改动后写 _trigger_key_override 立即生效
        try:
            from lingxi_ime.backend import ime_config as _cfg
            default_vk = getattr(_cfg, "PINYIN_TRIGGER_KEY", VK_RCONTROL)
        except Exception:
            default_vk = VK_RCONTROL

        def poll():
            was_down = False
            while True:
                trigger = getattr(self, "_trigger_key_override", default_vk)
                down = (ctypes.windll.user32.GetAsyncKeyState(trigger) & 0x8000) != 0
                if down and not was_down:
                    was_down = True
                    log(f"toggle: trigger {trigger:#x} pressed")
                    self._root.after(0, self._toggle)
                elif not down and was_down:
                    was_down = False
                time.sleep(0.03)
        threading.Thread(target=poll, daemon=True).start()
        log(f"toggle hotkey started (trigger={default_vk:#x})")

    def _toggle(self):
        self._active = not self._active
        log(f"toggle: active={self._active}")
        if self._active:
            self._input_str = ""
            self._candidates = []
            self._sentence = None
            self._set_composing(False)
            # 记录激活时的前台窗口作为上屏目标（候选窗不抢前台，故前台即目标）
            try:
                self._target_hwnd = ctypes.windll.user32.GetForegroundWindow()
                log(f"toggle: target hwnd={self._target_hwnd}")
            except Exception:
                self._target_hwnd = 0
        else:
            self._hide_candidates()
        self._refresh_bar()

    # 修饰键 vk：Ctrl(0xA2/0xA3/0x11) Shift(0xA0/0xA1/0x10) Alt(0xA4/0xA5/0x12) Win(0x5B/0x5C)
    _MODIFIER_VKS = frozenset({
        0x10, 0x11, 0x12, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0x5B, 0x5C,
    })

    def _in_inject_window(self):
        """是否处于注入后的时序保护窗口（150ms）内。"""
        return (time.time() - self._last_inject_ts) < 0.15

    def _wants_key(self, vk):
        """钩子线程里调用：仅判断是否要吞这个键。不碰 SQLite/Tk，必须极快。"""
        if vk in self._MODIFIER_VKS:
            return False
        # 用物理修饰键状态(钩子里跟踪真实按键,注入的 Ctrl+V 不影响),
        # 而非 GetAsyncKeyState——它会把注入残留的虚拟 Ctrl 误判为用户按下,
        # 导致上屏后紧随的字母被当成 Ctrl+字母 放行给 cursor(触发快捷键)。
        phys = getattr(self, "_phys_mod", None) or {}
        ctrl = phys.get("ctrl", False)
        alt = phys.get("alt", False)
        win = phys.get("win", False)
        if win:
            # Win 组合键(Win+D/E/R/L…)一律放行,绝不吞字母 → 系统快捷键不被吃掉
            return False
        if ctrl or alt:
            # 诊断：字母键被放行（可能误触快捷键），记录修饰键状态
            if 0x41 <= vk <= 0x5A:
                log(f"PASS-THRU letter vk={vk:#x} ({chr(vk)}) ctrl={ctrl} alt={alt} active={self._active}")
            return False
        # 字母
        if 0x41 <= vk <= 0x5A:
            return True
        # 数字/空格/回车/退格/翻页：只有正在组词或已有候选时才吞
        # 注意：钩子线程里只读原子布尔 _composing，绝不碰 _input_str/_candidates
        if VK_0 <= vk <= VK_9 or vk in (VK_SPACE, VK_RETURN, VK_BACK,
                                         VK_ESCAPE, 0x09, 0x21, 0x22, 0xBB, 0xBD):
            return bool(self._composing)
        # 标点键：激活时吞进来（按中英标点设置上屏），见 _on_key 标点分支
        if vk in self._PUNCT_VKS:
            return True
        return False

    # 常用标点 vk:, . ; ' / [ ] \ ` -
    _PUNCT_VKS = frozenset({0xBC, 0xBE, 0xBA, 0xDE, 0xBF, 0xDB, 0xDD, 0xDC, 0xC0})

    def _safe_on_key(self, vk):
        """主线程里执行：包一层 try，任何异常都不影响后续按键。"""
        try:
            self._on_key(vk)
        except Exception as e:
            log(f"_on_key ERROR vk={vk:#x}: {e}")
            import traceback
            log(traceback.format_exc())

    def _set_composing(self, value):
        """主线程维护组词标志，并刷新控制栏灯。"""
        self._composing = bool(value)
        self._refresh_bar()

    def _on_key(self, vk):
        log(f"_on_key: vk={vk:#x} input={self._input_str!r}")
        # 命令模式:字母/数字/空格直接累加进命令串,回车触发命令
        if self._cmd_mode:
            if 0x41 <= vk <= 0x5A or VK_0 <= vk <= VK_9:
                self._input_str += chr(vk).lower()
                self._show_candidates()
                return True
            if vk == VK_SPACE:
                self._input_str += " "  # 空格作为参数分隔符累加(如 /bt 1010)
                self._show_candidates()
                return True
            if vk == VK_BACK:
                self._input_str = self._input_str[:-1]
                if not self._input_str or self._input_str == "/":
                    # 删空退出命令模式
                    self._cmd_mode = False
                    self._input_str = ""
                    self._set_composing(False)
                    self._hide_candidates()
                else:
                    self._show_candidates()
                return True
            if vk == VK_RETURN:
                cmd = self._input_str
                self._cmd_mode = False
                self._input_str = ""
                self._set_composing(False)
                self._hide_candidates()
                log(f"cmd mode: run {cmd!r}")
                threading.Thread(target=self._prisirwork_rpc, args=(cmd, "cmd"), daemon=True).start()
                return True
            if vk == VK_ESCAPE:
                self._cmd_mode = False
                self._input_str = ""
                self._set_composing(False)
                self._hide_candidates()
                return True
            return True
        # 字母
        if 0x41 <= vk <= 0x5A:
            # Shift 临时上屏英文：按住 Shift 打字，字母直接上屏为大写英文，不进拼音
            if ctypes.windll.user32.GetKeyState(0x10) & 0x8000:
                # 若已有拼音输入，先清掉（避免残留）
                if self._input_str:
                    self._input_str = ""
                    self._candidates = []
                    self._set_composing(False)
                    self._hide_candidates()
                ch = chr(vk).upper()
                if self.full_width:
                    ch = self._to_fullwidth(ch)
                self._send_text(ch)
                log(f"shift-en: commit {ch}")
                return True
            self._input_str += chr(vk).lower()
            self._set_composing(True)
            # 字母键只正常查询（单字/词组/混拼），不自动进逐字选——
            # 否则打到一半凑够 3 音节就被强制拽进逐字选，打断输入。
            if self._sentence is not None:
                # 已在逐字选：继续输入视为放弃逐字选，退回普通编辑
                self._exit_sentence()
            self._query()
            self._show_candidates()
            return True
        # 数字 0-9（或 Shift+数字 上屏符号）
        if VK_0 <= vk <= VK_9:
            # 检测 Shift 状态
            shift_pressed = ctypes.windll.user32.GetKeyState(0x10) & 0x8000
            if shift_pressed:
                # Shift+数字 → 上屏符号
                symbols = "!@#$%^&*()"
                idx = vk - VK_0
                if 0 <= idx < len(symbols):
                    self._send_symbol(symbols[idx])
                return True
            # 普通数字：只有候选词非空时才选择，否则放行（直接输入数字）
            if self._candidates:
                idx = vk - VK_0 - 1
                log(f"number: idx={idx} candidates={len(self._candidates)}")
                if 0 <= idx < len(self._candidates):
                    self._selected = idx
                    if self._sentence is not None:
                        self._sentence_pick()
                    else:
                        self._commit()
                return True
            # 无候选词时，若是 >=3 音节整句，按数字也进入逐字选
            segs = self.engine.segment_pinyin(self._input_str)
            if len(segs) >= 3 and self._is_full_sentence(segs):
                self._enter_sentence(segs)
                self._sentence_query()
                self._show_candidates()
                return True
            # 无候选词时放行，让数字直接进记事本
            return False
        # Tab：整句时进入逐字选（智能首选不满足时手动逐字挑）
        if vk == 0x09:
            if self._sentence is None and self._is_sentence_input():
                segs = self.engine.segment_pinyin(self._input_str)
                self._enter_sentence(segs)
                self._sentence_query()
                self._show_candidates()
            return True
        # 空格
        if vk == VK_SPACE:
            log(f"space: candidates={len(self._candidates)}")
            if self._sentence is not None:
                if self._candidates:
                    self._sentence_pick()
                return True
            # 整句有智能首选时，空格上屏首选整句；否则有候选上屏候选
            if self._candidates:
                self._commit()
            return True
        # 翻页：PageDown / PageUp / + / -（只有候选词非空时才翻页）
        shift_pressed = ctypes.windll.user32.GetKeyState(0x10) & 0x8000
        if vk == 0x22 or (vk == 0xBB and not shift_pressed):  # PageDown 或 +
            if self._candidates:
                self._page_down()
                return True
            return False  # 无候选词时放行，让 = 直接输入
        if vk == 0x21 or (vk == 0xBD and not shift_pressed):  # PageUp 或 -
            if self._candidates:
                self._page_up()
                return True
            return False  # 无候选词时放行，让 - 直接输入
        # Shift + - / + → 上屏 _ / +
        if shift_pressed and vk == 0xBD:
            self._send_symbol("_")
            return True
        if shift_pressed and vk == 0xBB:
            self._send_symbol("+")
            return True
        # 回车
        if vk == VK_RETURN:
            log(f"return: input={self._input_str!r}")
            if self._sentence is not None:
                return True  # 逐字选模式下回车无效，避免上屏原始拼音
            if self._input_str:
                self._commit(raw=True)
            return True
        # 退格
        if vk == VK_BACK:
            log(f"backspace: input before={self._input_str!r}")
            if self._sentence is not None:
                self._sentence_backspace()
                return True
            if self._input_str:
                # 输入法还有字母，删输入法的
                self._input_str = self._input_str[:-1]
                log(f"backspace: input after={self._input_str!r}")
                if not self._input_str:
                    self._set_composing(False)
                self._query()
                self._show_candidates()
            else:
                # 输入法已空，转发给记事本删内容
                self._forward_backspace()
            return True
        # Esc
        if vk == VK_ESCAPE:
            self._exit_sentence()
            self._input_str = ""
            self._candidates = []
            self._set_composing(False)
            self._hide_candidates()
            return True
        # 标点键：按中英标点/全角设置上屏
        if vk in self._PUNCT_VKS:
            # / 在输入串为空时进入超级命令模式(打 /date /bt 1010 等),需在配置开启
            if vk == 0xBF and not self._input_str and not self._candidates and self._super_cmd_enabled():
                self._cmd_mode = True
                self._input_str = "/"
                self._set_composing(True)
                self._show_candidates()
                log("cmd mode: enter")
                return True
            # 有候选时 , . 作翻页键（主流输入法习惯）
            if self._candidates and vk == 0xBC:
                self._page_up(); return True
            if self._candidates and vk == 0xBE:
                self._page_down(); return True
            # 以词定字：候选是多字词时 [ 取首字 / ] 取尾字
            if vk in (0xDB, 0xDD) and self._candidates and self._selected < len(self._candidates):
                word = self._candidates[self._selected]
                if len(word) > 1:
                    char = word[0] if vk == 0xDB else word[-1]
                    self.engine.learn(self._input_str, char)
                    log(f"yicidingzi: vk={vk:#x} word={word!r} -> {char!r}")
                    self._commit_text_direct(char)
                    return True
            self._commit_punct(vk)
            return True
        return False

    # 标点映射：vk -> (中文标点, 英文标点)。Shift 态单独处理。
    _PUNCT_MAP = {
        0xBC: ("，", ","),   # ,
        0xBE: ("。", "."),   # .
        0xBA: ("；", ";"),   # ;
        0xDE: ("'", "'"),    # '（中文按 Shift 出"“”"，单击仍'）
        0xBF: ("、", "/"),   # /
        0xDB: ("【", "["),   # [
        0xDD: ("】", "]"),   # ]
        0xDC: ("、", "\\"),  # backslash
        0xC0: ("·", "`"),    # `
    }
    _PUNCT_MAP_SHIFT = {
        0xBC: ("《", "<"),   # Shift+,
        0xBE: ("》", ">"),   # Shift+.
        0xBA: ("：", ":"),   # Shift+;
        0xDE: ("“", "\""),   # Shift+'
        0xBF: ("？", "?"),   # Shift+/
        0xDB: ("「", "{"),   # Shift+[
        0xDD: ("」", "}"),   # Shift+]
    }

    def _commit_punct(self, vk):
        """上屏标点：chinese_punct 决定中/英，full_width 决定半/全角。"""
        shift = bool(ctypes.windll.user32.GetKeyState(0x10) & 0x8000)
        table = self._PUNCT_MAP_SHIFT if shift else self._PUNCT_MAP
        zh, en = table.get(vk, (None, None))
        if zh is None:
            return
        char = zh if self.chinese_punct else en
        if self.full_width and not self.chinese_punct:
            char = self._to_fullwidth(en)
        self._send_text(char)
        log(f"punct: vk={vk:#x} shift={shift} zh={self.chinese_punct} fw={self.full_width} -> {char!r}")

    @staticmethod
    def _to_fullwidth(s):
        """半角字符转全角（ASCII 可打印区 -> 全角区）。"""
        out = []
        for ch in s:
            code = ord(ch)
            if ch == " ":
                out.append("　")
            elif 0x21 <= code <= 0x7E:
                out.append(chr(code + 0xFEE0))
            else:
                out.append(ch)
        return "".join(out)

    # ---------------- 整句逐字选 ----------------
    def _is_full_sentence(self, segs):
        """切分结果是否覆盖整个输入串（即整句都是全拼，无残字母）。"""
        return bool(segs) and len("".join(segs)) == len(self._input_str)

    def _is_sentence_input(self):
        """当前输入是否为 >=3 音节的整句全拼（未进逐字选时的判断）。"""
        if not self._input_str:
            return False
        segs = self.engine.segment_pinyin(self._input_str)
        return len(segs) >= 3 and self._is_full_sentence(segs)

    def _enter_sentence(self, segs):
        """进入逐字选模式。"""
        self._sentence = {"segments": segs, "pos": 0, "picked": []}
        log(f"sentence: enter segments={segs}")

    def _exit_sentence(self):
        log("sentence: exit")
        self._sentence = None

    def _sentence_query(self):
        """逐字选模式：给当前音节的单字候选。"""
        s = self._sentence
        seg = s["segments"][s["pos"]]
        results = self.engine.query(seg, "pinyin")
        # 只保留单字候选（逐字选是逐字挑）
        words = [r[0] for r in results if len(r[0]) == 1]
        if not words:  # 该音节无单字，退化为全部候选
            words = [r[0] for r in results]
        self._all_candidates = words
        self._page = 0
        self._update_page()
        log(f"sentence: pos={s['pos']} seg={seg!r} -> {words[:5]}")

    def _sentence_pick(self):
        """逐字选模式：选定当前字，推进到下一音节；选完则整句上屏。"""
        s = self._sentence
        if not (self._candidates and self._selected < len(self._candidates)):
            return
        char = self._candidates[self._selected]
        self.engine.learn(s["segments"][s["pos"]], char)
        s["picked"].append(char)
        s["pos"] += 1
        log(f"sentence: picked={''.join(s['picked'])!r} pos={s['pos']}/{len(s['segments'])}")
        if s["pos"] >= len(s["segments"]):
            # 全部选完：整句一次上屏
            text = "".join(s["picked"])
            picked = text
            self._exit_sentence()
            self._input_str = ""
            self._candidates = []
            self._all_candidates = []
            self._set_composing(False)
            self._hide_candidates()
            log(f"sentence: commit {picked!r}")
            try:
                self._send_text(picked)
            except Exception as e:
                log(f"sentence: send failed {e}")
        else:
            self._sentence_query()
            self._show_candidates()

    def _sentence_backspace(self):
        """逐字选模式：回退到上一音节重选。"""
        s = self._sentence
        if s["pos"] > 0:
            s["pos"] -= 1
            s["picked"].pop()
            log(f"sentence: back to pos={s['pos']}")
            self._sentence_query()
            self._show_candidates()
        else:
            # 在第 0 个字再退：退出逐字选，回到普通编辑（清掉最后一字母）
            self._exit_sentence()
            self._input_str = self._input_str[:-1]
            self._query()
            self._show_candidates()

    def _query(self):
        if not self._input_str:
            self._candidates = []
            self._all_candidates = []
            self._page = 0
            return
        # 整句（>=3 音节全拼）：第一候选给"智能首选整句"，空格直接上屏；
        # 不混拼/单字查询，避免候选窗显示首音节单字（如 zhuzixuan 显示"逐"）误导。
        segs = self.engine.segment_pinyin(self._input_str)
        if len(segs) >= 3 and self._is_full_sentence(segs):
            smart = self.engine.smart_sentence(self._input_str)
            self._all_candidates = [smart] if smart else []
            self._page = 0
            self._update_page()
            log(f"query: sentence {self._input_str!r} segs={segs} smart={smart!r}")
            return
        # 先尝试整词查询
        results = self.engine.query(self._input_str, "pinyin")
        self._all_candidates = [r[0] for r in results]
        self._page = 0
        self._update_page()
        log(f"query: input={self._input_str!r} -> total={len(self._all_candidates)}")

    def _update_page(self):
        """更新当前页候选词"""
        start = self._page * self._cand_per_page
        end = start + self._cand_per_page
        self._candidates = self._all_candidates[start:end]
        self._selected = 0

    def _page_down(self):
        """下一页"""
        if (self._page + 1) * self._cand_per_page < len(self._all_candidates):
            self._page += 1
            self._update_page()
            self._show_candidates()
            log(f"page_down: page={self._page}")

    def _page_up(self):
        """上一页"""
        if self._page > 0:
            self._page -= 1
            self._update_page()
            self._show_candidates()
            log(f"page_up: page={self._page}")

    def _show_candidates(self):
        # 输入为空时只隐藏，不销毁
        if not self._input_str:
            self._hide_candidates()
            return

        x, y = self.context.get_caret_position()
        if x == 0 and y == 0:
            import pyautogui
            x, y = pyautogui.position()
            y += 30

        if self._candidate_win is None:
            self._candidate_win = tk.Toplevel(self._root)
            self._candidate_win.overrideredirect(True)
            self._candidate_win.attributes("-topmost", True)
            self._candidate_win.configure(bg=self._ui_bg)
            # 关键：候选窗永不抢焦点（WS_EX_NOACTIVATE + WS_EX_TOOLWINDOW）
            # 这样鼠标点击候选词时，键盘焦点始终在记事本，Ctrl+V 直达
            self._set_no_activate(self._candidate_win)

            # 第一行：显示输入字母
            self._input_label = tk.Label(
                self._candidate_win,
                text="",
                font=(self._font_family, self._input_font_size, "bold"),
                bg=self._ui_bg_input,
                fg=self._ui_fg_input,
                anchor="w",
                padx=8,
                pady=2
            )
            self._input_label.pack(fill=tk.X)

            # 第二行：候选词（横排/竖排，按配置）
            cand_frame = tk.Frame(self._candidate_win, bg=self._ui_bg)
            cand_frame.pack(fill=tk.X)
            self._cand_labels = []
            pack_side = tk.LEFT if self._layout == "horizontal" else tk.TOP
            for i in range(self._cand_per_page):
                lbl = tk.Label(cand_frame, text="",
                               font=(self._font_family, self._font_size),
                               bg=self._ui_bg, fg=self._ui_fg_cand,
                               anchor="w", padx=8, pady=2)
                if self._layout == "horizontal":
                    lbl.pack(side=tk.LEFT)
                else:
                    lbl.pack(side=tk.TOP, fill=tk.X)
                self._cand_labels.append(lbl)

        # 更新输入字母显示
        if self._sentence is not None:
            # 逐字选模式：显示 已选字 + 剩余音节 进度
            s = self._sentence
            done = "".join(s["picked"])
            rest = "'".join(s["segments"][s["pos"]:])
            self._input_label.config(text=f"{done} | {rest}")
        elif self._is_sentence_input():
            # 整句：显示分音节拼音 + 提示（空格上屏智能首选 / Tab 逐字选）
            segs = self.engine.segment_pinyin(self._input_str)
            self._input_label.config(text="'".join(segs) + "  [空格上屏·Tab逐字选]")
        else:
            self._input_label.config(text=self._input_str)

        for i, lbl in enumerate(self._cand_labels):
            if i < len(self._candidates):
                prefix = f"{i+1}. " if i == self._selected else f"{i+1}  "
                color = self._ui_fg_sel if i == self._selected else self._ui_fg_cand
                lbl.config(text=prefix + self._candidates[i], fg=color)
                # 绑定鼠标点击
                lbl.bind("<Button-1>", lambda e, idx=i: self._on_candidate_click(idx))
            else:
                lbl.config(text="", fg=self._ui_fg_cand)

        self._candidate_win.geometry(f"+{x}+{y + 20}")
        self._candidate_win.deiconify()
        log(f"show_candidates: at ({x},{y})")

    def _set_no_activate(self, win):
        """给候选窗加 WS_EX_NOACTIVATE，鼠标点击不抢焦点"""
        try:
            win.update_idletasks()
            hwnd = int(win.wm_frame(), 16)
            GWL_EXSTYLE = -20
            WS_EX_NOACTIVATE = 0x08000000
            WS_EX_TOOLWINDOW = 0x00000080
            user32 = ctypes.windll.user32
            style = user32.GetWindowLongW(hwnd, GWL_EXSTYLE)
            user32.SetWindowLongW(hwnd, GWL_EXSTYLE, style | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW)
            log(f"_set_no_activate: hwnd={hwnd} ok")
        except Exception as e:
            log(f"_set_no_activate failed: {e}")

    def _on_candidate_click(self, idx):
        """鼠标点击候选词"""
        log(f"click: idx={idx}, candidates={len(self._candidates)}, selected={self._selected}")
        if 0 <= idx < len(self._candidates):
            self._selected = idx
            if self._sentence is not None:
                self._sentence_pick()
            else:
                self._commit()
        else:
            log(f"click: idx {idx} out of range")

    def _send_symbol(self, symbol):
        """直接上屏符号（Shift+数字）"""
        log(f"send_symbol: {symbol}")
        self._injecting = True
        try:
            import pyperclip
            pyperclip.copy(symbol)
            time.sleep(0.05)
            ctypes.windll.user32.keybd_event(0x11, 0, 0, 0)
            ctypes.windll.user32.keybd_event(0x56, 0, 0, 0)
            ctypes.windll.user32.keybd_event(0x56, 0, 2, 0)
            ctypes.windll.user32.keybd_event(0x11, 0, 2, 0)
            log(f"send_symbol: ok")
        except Exception as e:
            log(f"send_symbol failed: {e}")
        finally:
            self._injecting = False

    def _forward_backspace(self):
        """转发 Backspace 到目标窗口（记事本）"""
        log("forward_backspace: send to notepad")
        self._injecting = True
        try:
            user32 = ctypes.windll.user32
            user32.keybd_event(0x08, 0, 0, 0)  # Backspace down
            user32.keybd_event(0x08, 0, 2, 0)  # Backspace up
        except Exception as e:
            log(f"forward_backspace failed: {e}")
        finally:
            self._injecting = False

    def _hide_candidates(self):
        if self._candidate_win:
            self._candidate_win.withdraw()

    def _commit(self, raw=False):
        log(f"_commit: raw={raw} input={self._input_str!r} candidates={self._candidates[:3] if self._candidates else []}")
        if raw:
            text = self._input_str
        elif self._candidates and self._selected < len(self._candidates):
            text = self._candidates[self._selected]
            self.engine.learn(self._input_str, text)
        else:
            text = self._input_str

        log(f"_commit: text={text!r} about_to_send")
        if text:
            try:
                self._send_text(text)
                log(f"_commit: _send_text returned")
            except Exception as e:
                log(f"_commit: _send_text FAILED: {e}")
                import traceback
                log(traceback.format_exc())

        self._input_str = ""
        self._candidates = []
        self._set_composing(False)
        self._hide_candidates()

    def _commit_text_direct(self, text):
        """直接上屏指定文本并清理输入状态（用于以词定字等不走候选/原始的提交）。"""
        log(f"_commit_text_direct: {text!r}")
        try:
            self._send_text(text)
        except Exception as e:
            log(f"_commit_text_direct failed: {e}")
        self._input_str = ""
        self._candidates = []
        self._set_composing(False)
        self._hide_candidates()

    def _probe_target_sel(self, tag, hwnd):
        """Bug1 遥测:读目标窗口焦点控件的类名 + EM_GETSEL 选中态写日志。

        根因已定位并修复(R1 _collapse_sel_to_end),探针默认静默 — 设环境变量
        PRISIR_IME_DEBUG_SEL=1 才输出,用于日后回归/再诊断。始终 try/except 包裹,
        绝不影响上屏主路径。
        """
        if not os.environ.get("PRISIR_IME_DEBUG_SEL"):
            return 0, ""
        try:
            EM_GETSEL = 0x00B0
            WM_GETTEXT = 0x000D
            u = ctypes.windll.user32
            focus = u.GetFocus()
            # GetFocus 只对本线程有效,目标对话框是别的进程 → 用 GetGUIThreadInfo 拿焦点
            gui = GUITHREADINFO_PROBE()
            gui.cbSize = ctypes.sizeof(GUITHREADINFO_PROBE)
            tid = u.GetWindowThreadProcessId(hwnd, None)
            real_focus = 0
            if tid and u.GetGUIThreadInfo(tid, ctypes.byref(gui)):
                real_focus = gui.hwndFocus or 0
            target = real_focus or focus or hwnd
            cbuf = ctypes.create_unicode_buffer(64)
            u.GetClassNameW(target, cbuf, 64)
            cls = cbuf.value
            s = wintypes.DWORD(0); e = wintypes.DWORD(0)
            u.SendMessageW(target, EM_GETSEL, ctypes.byref(s), ctypes.byref(e))
            tbuf = ctypes.create_unicode_buffer(256)
            u.SendMessageW(target, WM_GETTEXT, 256, tbuf)
            log(f"[bug1-probe][{tag}] hwnd={hwnd:#x} focus={target:#x} cls={cls!r} "
                f"sel=[{s.value},{e.value}) text={tbuf.value!r}")
            return target, cls
        except Exception as ex:
            log(f"[bug1-probe][{tag}] failed: {ex}")
            return 0, ""

    def _collapse_sel_to_end(self, hwnd):
        """R1 修复:上屏前把目标 EDIT 的选中态归位到末尾(取消全选)。

        Bug1 根因(遥测实锤): 壳(Electron)导出的 IFileSaveDialog 在每次上屏后
        把文件名框内容**自发重新全选**(sel 从 [n,n) 跳回 [0,n)),于是下一次粘贴
        永远替换掉上一次内容 — 「文件」→「命名为」→「导出」逐次替换。真 TSF commit
        会抑制这个重选,裸粘贴不会。

        对策: 粘贴前 EM_SETSEL(len,len) 把光标强制归末、取消选中,粘贴即从末尾
        追加而非替换。仅对 Edit 类控件生效(标准 EDIT 才认 EM_SETSEL);非 Edit 控件
        (如 DirectUI 内嵌)跳过,交给默认行为。try/except 包裹,失败不挡上屏。
        """
        try:
            EM_GETSEL = 0x00B0
            EM_SETSEL = 0x00B1
            WM_GETTEXT = 0x000D
            WM_GETTEXTLENGTH = 0x000E
            u = ctypes.windll.user32
            # 复用探针的焦点定位逻辑
            gui = GUITHREADINFO_PROBE()
            gui.cbSize = ctypes.sizeof(GUITHREADINFO_PROBE)
            tid = u.GetWindowThreadProcessId(hwnd, None)
            target = 0
            if tid and u.GetGUIThreadInfo(tid, ctypes.byref(gui)):
                target = gui.hwndFocus or 0
            if not target:
                return
            cbuf = ctypes.create_unicode_buffer(64)
            u.GetClassNameW(target, cbuf, 64)
            if cbuf.value != "Edit":
                return  # 非标准 Edit,不碰(避免误伤 DirectUI/自定义控件)
            s = wintypes.DWORD(0); e = wintypes.DWORD(0)
            u.SendMessageW(target, EM_GETSEL, ctypes.byref(s), ctypes.byref(e))
            if e.value > s.value:  # 有选中态才归位(全选/部分选中)
                ln = u.SendMessageW(target, WM_GETTEXTLENGTH, 0, 0)
                u.SendMessageW(target, EM_SETSEL, ln, ln)
                log(f"[bug1-fix] 归位光标: Edit sel [{s.value},{e.value}) -> [{ln},{ln})")
        except Exception as ex:
            log(f"[bug1-fix] failed (non-fatal): {ex}")

    def _send_text(self, text):
        """上屏：SendInput 真实 Ctrl+V（兼容性最强），发完记录时间戳供钩子做时序保护。

        WM_PASTE 对 Cursor/Electron 等非标准 Edit 控件无效（GetFocus 拿不到真编辑框，
        ok=1 只是投递成功、实际没粘贴），故主路径仍用 SendInput。
        Ctrl 残留竞态改由 _wants_key 的"注入后时序窗口"解决，见 _wants_key。
        """
        log(f"_send_text: {text!r}")
        user32 = ctypes.windll.user32
        self._injecting = True
        try:
            hwnd = user32.GetForegroundWindow()
            if self._is_own_window(hwnd):
                hwnd = self._target_hwnd or hwnd
            log(f"_send_text: target hwnd={hwnd}")
            import pyperclip
            pyperclip.copy(text)
            time.sleep(0.05)
            self._force_foreground(hwnd)
            time.sleep(0.05)
            self._probe_target_sel("before", hwnd)  # Bug1 遥测:上屏前选中态
            self._collapse_sel_to_end(hwnd)  # Bug1 R1 修复:取消全选,光标归末再粘贴
            self._send_ctrl_v()
            # 记录注入结束时间：钩子在此后短时间窗口内，把读到的 Ctrl 视为注入残留
            self._last_inject_ts = time.time()
            self._wait_ctrl_released()
            time.sleep(0.05)
            self._probe_target_sel("after", hwnd)  # Bug1 遥测:上屏后选中态
            log("_send_text: Ctrl+V sent")
        except Exception as e:
            log(f"_send_text failed: {e}")
            import traceback
            log(traceback.format_exc())
        finally:
            self._injecting = False

    def _wait_ctrl_released(self):
        """等到物理 Ctrl 真正弹起（最多 200ms），避免注入的 Ctrl 残留污染后续按键判断。"""
        user32 = ctypes.windll.user32
        for _ in range(40):
            if not (user32.GetAsyncKeyState(0x11) & 0x8000):
                return
            time.sleep(0.005)

    def _is_own_window(self, hwnd):
        """判断 hwnd 是否属于本进程（候选窗/控制栏/托盘菜单等），是则不作为上屏目标。"""
        if not hwnd:
            return False
        try:
            pid = wintypes.DWORD()
            ctypes.windll.user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
            return pid.value == os.getpid()
        except Exception:
            return False

    def _force_foreground(self, hwnd):
        """强制把 hwnd 设为前台并聚焦。"""
        user32 = ctypes.windll.user32
        k32 = ctypes.windll.kernel32
        try:
            cur = k32.GetCurrentThreadId()
            tgt = user32.GetWindowThreadProcessId(hwnd, None)
            if cur != tgt:
                user32.AttachThreadInput(cur, tgt, True)
            user32.SetForegroundWindow(hwnd)
            user32.SetFocus(hwnd)
            if cur != tgt:
                user32.AttachThreadInput(cur, tgt, False)
            log(f"_force_foreground: hwnd={hwnd} ok")
        except Exception as e:
            log(f"_force_foreground failed: {e}")

    def _send_ctrl_v(self):
        """用 SendInput 发一次真实 Ctrl+V（标准 MOUSEINPUT/KEYBDINPUT union）。"""
        user32 = ctypes.windll.user32
        INPUT_KEYBOARD = 1
        KEYEVENTF_KEYUP = 0x0002
        ULONG_PTR = wintypes.WPARAM

        class MOUSEINPUT(ctypes.Structure):
            _fields_ = [("dx", wintypes.LONG), ("dy", wintypes.LONG),
                        ("mouseData", wintypes.DWORD), ("dwFlags", wintypes.DWORD),
                        ("time", wintypes.DWORD), ("dwExtraInfo", ULONG_PTR)]

        class KEYBDINPUT(ctypes.Structure):
            _fields_ = [("wVk", wintypes.WORD), ("wScan", wintypes.WORD),
                        ("dwFlags", wintypes.DWORD), ("time", wintypes.DWORD),
                        ("dwExtraInfo", ULONG_PTR)]

        class HARDWAREINPUT(ctypes.Structure):
            _fields_ = [("uMsg", wintypes.DWORD), ("wParamL", wintypes.WORD),
                        ("wParamH", wintypes.WORD)]

        class _INPUTunion(ctypes.Union):
            _fields_ = [("mi", MOUSEINPUT), ("ki", KEYBDINPUT), ("hi", HARDWAREINPUT)]

        class INPUT(ctypes.Structure):
            _fields_ = [("type", wintypes.DWORD), ("union", _INPUTunion)]

        def mk(vk, flags):
            inp = INPUT()
            inp.type = INPUT_KEYBOARD
            inp.union.ki = KEYBDINPUT(vk, 0, flags, 0, 0)
            return inp

        VK_CTRL, VK_V = 0x11, 0x56
        seq = (INPUT * 4)(
            mk(VK_CTRL, 0), mk(VK_V, 0),
            mk(VK_V, KEYEVENTF_KEYUP), mk(VK_CTRL, KEYEVENTF_KEYUP),
        )
        sent = user32.SendInput(4, seq, ctypes.sizeof(INPUT))
        log(f"_send_ctrl_v: SendInput sent={sent} sizeof={ctypes.sizeof(INPUT)}")


def main():
    app = PinyinIME()
    app.start()


if __name__ == "__main__":
    main()
