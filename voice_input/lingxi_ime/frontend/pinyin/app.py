# -*- coding: utf-8 -*-
"""灵犀·拼音 — 外挂式输入法
全局键盘钩子捕获输入 → 光标位置候选窗 → SendInput 上屏
"""
import tkinter as tk
from tkinter import ttk
import threading
import time
import sys
import os
import ctypes
from ctypes import wintypes

# 添加父目录到路径
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from backend.engine import get_engine
from backend.context import get_context
from backend.output import get_output

# Windows 常量
WH_KEYBOARD_LL = 13
WM_KEYDOWN = 0x0100
WM_KEYUP = 0x0101
WM_SYSKEYDOWN = 0x0104
WM_SYSKEYUP = 0x0105
VK_BACK = 0x08
VK_RETURN = 0x0D
VK_ESCAPE = 0x1B
VK_SPACE = 0x20
VK_0 = 0x30
VK_9 = 0x39

# 输入方案
SCHEME_QUANPIN = "quanpin"  # 全拼
SCHEME_SHUANGPIN = "shuangpin"  # 双拼


class KBDLLHOOKSTRUCT(ctypes.Structure):
    _fields_ = [
        ("vkCode", wintypes.DWORD),
        ("scanCode", wintypes.DWORD),
        ("flags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong)),
    ]


class PinyinIME:
    """灵犀·拼音外挂式输入法"""

    def __init__(self):
        self.engine = get_engine()
        self.context = get_context()
        self.output = get_output()

        self._root = None
        self._input_str = ""
        self._candidates = []
        self._selected_index = 0
        self._target_hwnd = 0

        # 状态
        self._active = False
        self._hook = None
        self._hook_proc = None
        self._scheme = SCHEME_QUANPIN  # 默认全拼

        # 候选窗
        self._candidate_win = None
        self._tray = None

    def start(self):
        """启动输入法"""
        # 创建隐藏的 Tk 根窗口（用于候选窗）
        self._root = tk.Tk()
        self._root.withdraw()

        # 创建系统托盘
        self._create_tray()

        # 创建设置窗口
        self._create_settings_window()

        # 安装全局键盘钩子
        if not self._install_hook():
            print("[ERROR] 键盘钩子安装失败")
            return

        # 启动右 Ctrl 切换热键
        self._start_toggle_hotkey()

        print("[OK] 灵犀·拼音外挂式已启动(全局钩子)")
        self._root.mainloop()

    def _start_toggle_hotkey(self):
        """启动右 Ctrl 切换热键"""
        import ctypes
        user32 = ctypes.windll.user32
        VK_RCONTROL = 0xA3

        def poll():
            was_down = False
            while True:
                try:
                    down = (user32.GetAsyncKeyState(VK_RCONTROL) & 0x8000) != 0
                    if down and not was_down:
                        was_down = True
                        self._toggle()
                    elif not down and was_down:
                        was_down = False
                    time.sleep(0.03)
                except Exception:
                    time.sleep(0.1)

        threading.Thread(target=poll, daemon=True).start()

    def _toggle(self):
        """切换输入法激活状态（线程安全：调度到主线程）"""
        self._root.after(0, self._toggle_gui)

    def _toggle_gui(self):
        """在主线程切换 GUI 状态"""
        self._active = not self._active
        if self._active:
            self._target_hwnd = self.context.get_foreground_window()
            self._status_var.set("状态: 已激活(全拼)" if self._scheme == SCHEME_QUANPIN else "状态: 已激活(双拼)")
            self._input_str = ""
            self._candidates = []
        else:
            self._status_var.set("状态: 未激活")
            self._hide_candidates()
            if self._input_str:
                self._commit()

    def _create_tray(self):
        """创建系统托盘图标"""
        try:
            import pystray
            from PIL import Image, ImageDraw

            # 创建图标
            img = Image.new("RGB", (64, 64), "#ffffff")
            d = ImageDraw.Draw(img)
            d.ellipse([8, 8, 56, 56], fill="#128a4a")
            d.text((22, 18), "拼", fill="#ffffff", font=None)

            menu = pystray.Menu(
                pystray.MenuItem("打开设置", self._show_settings),
                pystray.MenuItem("退出", self._quit)
            )
            self._tray = pystray.Icon("lingxi_pinyin", img, "灵犀·拼音", menu)
            threading.Thread(target=self._tray.run, daemon=True).start()
        except Exception as e:
            print(f"[WARN] 托盘创建失败: {e}")

    def _create_settings_window(self):
        """创建设置窗口"""
        self._settings_win = tk.Toplevel(self._root)
        self._settings_win.title("灵犀·拼音 设置")
        self._settings_win.geometry("300x200")
        self._settings_win.withdraw()  # 默认隐藏

        # 输入方案选择
        tk.Label(self._settings_win, text="输入方案:", font=("Microsoft YaHei UI", 11)).pack(anchor="w", padx=10, pady=5)

        self._scheme_var = tk.StringVar(value=self._scheme)
        tk.Radiobutton(
            self._settings_win,
            text="全拼",
            variable=self._scheme_var,
            value=SCHEME_QUANPIN,
            font=("Microsoft YaHei UI", 10)
        ).pack(anchor="w", padx=20)
        tk.Radiobutton(
            self._settings_win,
            text="双拼",
            variable=self._scheme_var,
            value=SCHEME_SHUANGPIN,
            font=("Microsoft YaHei UI", 10)
        ).pack(anchor="w", padx=20)

        # 状态显示
        self._status_var = tk.StringVar(value="状态: 未激活")
        tk.Label(
            self._settings_win,
            textvariable=self._status_var,
            font=("Microsoft YaHei UI", 10),
            fg="#5b6b7c"
        ).pack(anchor="w", padx=10, pady=10)

        # 说明
        tk.Label(
            self._settings_win,
            text="按 右Ctrl 切换输入法\n直接打字出候选词",
            font=("Microsoft YaHei UI", 9),
            fg="#9aa3b2",
            justify="left"
        ).pack(anchor="w", padx=10, pady=5)

    def _show_settings(self):
        """显示设置窗口"""
        self._settings_win.deiconify()
        self._settings_win.lift()

    def _quit(self):
        """退出"""
        if self._tray:
            self._tray.stop()
        self._root.quit()

    def _install_hook(self):
        """安装全局键盘钩子"""
        user32 = ctypes.windll.user32
        k32 = ctypes.windll.kernel32

        k32.GetModuleHandleW.restype = wintypes.HMODULE
        user32.SetWindowsHookExW.restype = wintypes.HHOOK
        user32.CallNextHookEx.restype = ctypes.c_long
        user32.CallNextHookEx.argtypes = [wintypes.HHOOK, ctypes.c_int, wintypes.WPARAM, wintypes.LPARAM]

        HOOKPROC = ctypes.WINFUNCTYPE(ctypes.c_long, ctypes.c_int, wintypes.WPARAM, wintypes.LPARAM)
        user32.SetWindowsHookExW.argtypes = [ctypes.c_int, HOOKPROC, wintypes.HMODULE, wintypes.DWORD]

        hmod = k32.GetModuleHandleW(None)

        def _hook_proc(nCode, wParam, lParam):
            if nCode == 0:
                kb = ctypes.cast(lParam, ctypes.POINTER(KBDLLHOOKSTRUCT)).contents
                # 只处理 keydown
                if wParam in (WM_KEYDOWN, WM_SYSKEYDOWN):
                    # 只有激活状态才处理按键
                    if self._active:
                        # 处理按键并阻止传递(返回1=吞掉)
                        if self._on_keydown(kb.vkCode):
                            return 1
            return user32.CallNextHookEx(self._hook, nCode, wParam, lParam)

        self._hook_proc = HOOKPROC(_hook_proc)
        self._hook = user32.SetWindowsHookExW(WH_KEYBOARD_LL, self._hook_proc, hmod, 0)
        if not self._hook:
            err = k32.GetLastError()
            print(f"[ERROR] SetWindowsHookExW failed: {err}")
            return False
        return True

    def _on_keydown(self, vk_code):
        """处理键盘按下，返回 True=吞掉按键，False=放行"""
        self._log(f"[DEBUG] _on_keydown: vk={vk_code:#x}, active={self._active}, input={self._input_str!r}")

        # 字母键
        if 0x41 <= vk_code <= 0x5A:  # A-Z
            char = chr(vk_code).lower()
            self._input_str += char
            self._query()
            self._show_candidates()
            return True

        # 数字键 1-9（选择候选词）
        if VK_0 + 1 <= vk_code <= VK_9:
            idx = vk_code - VK_0 - 1
            self._log(f"[DEBUG] number key: idx={idx}, candidates={len(self._candidates)}")
            if 0 <= idx < len(self._candidates):
                self._selected_index = idx
                self._commit()
            return True

        # 空格（上屏首选）
        if vk_code == VK_SPACE:
            self._log(f"[DEBUG] space: candidates={len(self._candidates)}")
            if self._candidates:
                self._commit()
            return True

        # 回车（上屏原输入）
        if vk_code == VK_RETURN:
            self._log(f"[DEBUG] return: input={self._input_str!r}")
            if self._input_str:
                self._commit(raw=True)
            return True

        # 退格
        if vk_code == VK_BACK:
            if self._input_str:
                self._input_str = self._input_str[:-1]
                self._query()
                self._show_candidates()
            return True

        # Esc（取消）
        if vk_code == VK_ESCAPE:
            self._input_str = ""
            self._candidates = []
            self._hide_candidates()
            return True

        # 其他键放行
        return False

    def _query(self):
        """查询候选词"""
        if not self._input_str:
            self._candidates = []
            return
        # 先尝试整词查询
        results = self.engine.query(self._input_str, "pinyin")
        if results:
            self._candidates = [r[0] for r in results[:9]]
        else:
            # 整词无结果，尝试切分后查询
            segments = self.engine.segment_pinyin(self._input_str)
            if len(segments) > 1:
                # 多字：查询每个字的候选，取第一个字的候选展示
                # 用户继续输入会逐步缩小范围
                results = self.engine.query(segments[0], "pinyin")
                self._candidates = [r[0] for r in results[:9]]
            else:
                self._candidates = []
        self._selected_index = 0
        self._log(f"[DEBUG] _query: input={self._input_str!r}, candidates={len(self._candidates)}")

    def _show_candidates(self):
        """在光标位置显示候选窗"""
        if not self._input_str or not self._candidates:
            self._hide_candidates()
            return

        # 获取光标位置，失败则用鼠标位置
        x, y = self.context.get_caret_position()
        if x == 0 and y == 0:
            # fallback: 用鼠标位置
            import pyautogui
            x, y = pyautogui.position()
            y += 30

        if self._candidate_win is None:
            self._candidate_win = tk.Toplevel(self._root)
            self._candidate_win.overrideredirect(True)
            self._candidate_win.attributes("-topmost", True)
            self._candidate_win.configure(bg="#ffffff")

            self._cand_labels = []
            for i in range(9):
                lbl = tk.Label(
                    self._candidate_win,
                    text="",
                    font=("Microsoft YaHei UI", 12),
                    bg="#ffffff",
                    fg="#5b6b7c",
                    anchor="w",
                    padx=8,
                    pady=2
                )
                lbl.pack(side=tk.LEFT)  # 横向排列
                self._cand_labels.append(lbl)

        # 更新候选词
        for i, lbl in enumerate(self._cand_labels):
            if i < len(self._candidates):
                prefix = f"{i+1}. " if i == self._selected_index else f"{i+1}  "
                color = "#c47f2a" if i == self._selected_index else "#5b6b7c"
                lbl.config(text=prefix + self._candidates[i], fg=color)
            else:
                lbl.config(text="", fg="#5b6b7c")

        # 移动到光标位置
        self._candidate_win.geometry(f"+{x}+{y + 20}")
        self._candidate_win.deiconify()

    def _hide_candidates(self):
        """隐藏候选窗"""
        if self._candidate_win:
            self._candidate_win.withdraw()

    def _commit(self, raw=False):
        """上屏"""
        self._log(f"[DEBUG] _commit: raw={raw}, input={self._input_str!r}, candidates={self._candidates[:3] if self._candidates else []}")
        if raw:
            text = self._input_str
        elif self._candidates and self._selected_index < len(self._candidates):
            text = self._candidates[self._selected_index]
            self.engine.learn(self._input_str, text)
        else:
            text = self._input_str

        self._log(f"[DEBUG] _commit: text={text!r}, target={self._target_hwnd}")
        if text:
            # 使用 SendInput 发送 Unicode 字符
            try:
                self._send_unicode(text)
                self._log(f"[DEBUG] _commit: SendInput ok")
            except Exception as e:
                self._log(f"[ERROR] _commit: SendInput failed: {e}")
                # fallback: 剪贴板
                try:
                    import pyperclip
                    pyperclip.copy(text)
                    time.sleep(0.05)
                    ctypes.windll.user32.keybd_event(0x11, 0, 0, 0)
                    ctypes.windll.user32.keybd_event(0x56, 0, 0, 0)
                    ctypes.windll.user32.keybd_event(0x56, 0, 2, 0)
                    ctypes.windll.user32.keybd_event(0x11, 0, 2, 0)
                    self._log(f"[DEBUG] _commit: clipboard paste ok")
                except Exception as e2:
                    self._log(f"[ERROR] _commit: clipboard also failed: {e2}")

        # 重置
        self._input_str = ""
        self._candidates = []
        self._hide_candidates()

    def _send_unicode(self, text):
        """使用 SendInput 发送 Unicode 字符"""
        user32 = ctypes.windll.user32

        class KEYBDINPUT(ctypes.Structure):
            _fields_ = [
                ("wVk", wintypes.WORD),
                ("wScan", wintypes.WORD),
                ("dwFlags", wintypes.DWORD),
                ("time", wintypes.DWORD),
                ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong)),
            ]

        class INPUT(ctypes.Structure):
            class _INPUT(ctypes.Union):
                _fields_ = [("ki", KEYBDINPUT)]
            _fields_ = [
                ("type", wintypes.DWORD),
                ("_input", _INPUT),
            ]

        KEYEVENTF_UNICODE = 0x0004
        KEYEVENTF_KEYUP = 0x0002

        inputs = []
        for char in text:
            # 按下
            inp = INPUT()
            inp.type = 1
            inp._input.ki.wVk = 0
            inp._input.ki.wScan = ord(char)
            inp._input.ki.dwFlags = KEYEVENTF_UNICODE
            inp._input.ki.time = 0
            inp._input.ki.dwExtraInfo = None
            inputs.append(inp)

            # 松开
            inp = INPUT()
            inp.type = 1
            inp._input.ki.wVk = 0
            inp._input.ki.wScan = ord(char)
            inp._input.ki.dwFlags = KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
            inp._input.ki.time = 0
            inp._input.ki.dwExtraInfo = None
            inputs.append(inp)

        n = len(inputs)
        arr = (INPUT * n)(*inputs)
        user32.SendInput(n, arr, ctypes.sizeof(INPUT))

    def _log(self, msg):
        """写日志到文件"""
        try:
            log_file = os.path.join(os.path.dirname(os.path.abspath(__file__)), "pinyin_debug.log")
            with open(log_file, "a", encoding="utf-8") as f:
                f.write(f"{time.strftime('%H:%M:%S')} {msg}\n")
        except Exception:
            pass


def main():
    app = PinyinIME()
    app.start()


if __name__ == "__main__":
    main()
