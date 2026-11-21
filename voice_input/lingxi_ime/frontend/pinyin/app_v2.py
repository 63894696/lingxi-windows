# -*- coding: utf-8 -*-
"""灵犀·拼音 v2 — 复刻影子输入法核心逻辑
简化版：全局钩子 → 候选窗 → SendInput 上屏
"""
import tkinter as tk
import threading
import time
import sys
import os
import ctypes
from ctypes import wintypes

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from backend.engine import get_engine
from backend.context import get_context

# Windows 常量
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


class KBDLLHOOKSTRUCT(ctypes.Structure):
    _fields_ = [
        ("vkCode", wintypes.DWORD),
        ("scanCode", wintypes.DWORD),
        ("flags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong)),
    ]


class PinyinIME:
    def __init__(self):
        self.engine = get_engine()
        self.context = get_context()
        self._root = None
        self._input_str = ""
        self._candidates = []
        self._selected = 0
        self._active = False
        self._hook = None
        self._hook_proc = None
        self._candidate_win = None
        self._tray = None
        self._settings_win = None
        self._status_var = None
        self._scheme_var = None

    def start(self):
        self._root = tk.Tk()
        self._root.withdraw()
        self._create_tray()
        self._create_settings()
        self._install_hook()
        self._start_toggle_hotkey()
        print("[OK] 灵犀·拼音 v2 已启动")
        self._root.mainloop()

    def _create_tray(self):
        try:
            import pystray
            from PIL import Image, ImageDraw
            img = Image.new("RGB", (64, 64), "#ffffff")
            d = ImageDraw.Draw(img)
            d.ellipse([8, 8, 56, 56], fill="#128a4a")
            d.text((22, 18), "拼", fill="#ffffff")
            menu = pystray.Menu(
                pystray.MenuItem("设置", self._show_settings),
                pystray.MenuItem("退出", self._quit)
            )
            self._tray = pystray.Icon("lingxi_pinyin", img, "灵犀·拼音", menu)
            threading.Thread(target=self._tray.run, daemon=True).start()
        except Exception as e:
            print(f"[WARN] tray failed: {e}")

    def _create_settings(self):
        self._settings_win = tk.Toplevel(self._root)
        self._settings_win.title("灵犀·拼音 设置")
        self._settings_win.geometry("300x200")
        self._settings_win.withdraw()

        tk.Label(self._settings_win, text="输入方案:", font=("Microsoft YaHei UI", 11)).pack(anchor="w", padx=10, pady=5)
        self._scheme_var = tk.StringVar(value="quanpin")
        tk.Radiobutton(self._settings_win, text="全拼", variable=self._scheme_var, value="quanpin").pack(anchor="w", padx=20)
        tk.Radiobutton(self._settings_win, text="双拼", variable=self._scheme_var, value="shuangpin").pack(anchor="w", padx=20)

        self._status_var = tk.StringVar(value="状态: 未激活")
        tk.Label(self._settings_win, textvariable=self._status_var, font=("Microsoft YaHei UI", 10)).pack(anchor="w", padx=10, pady=10)

        tk.Label(self._settings_win, text="按 右Ctrl 切换输入法", font=("Microsoft YaHei UI", 9), fg="#9aa3b2").pack(anchor="w", padx=10, pady=5)

    def _show_settings(self):
        self._settings_win.deiconify()
        self._settings_win.lift()

    def _quit(self):
        if self._tray:
            self._tray.stop()
        self._root.quit()

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

        def _hook_proc(nCode, wParam, lParam):
            if nCode == 0 and self._active:
                kb = ctypes.cast(lParam, ctypes.POINTER(KBDLLHOOKSTRUCT)).contents
                if wParam in (WM_KEYDOWN, WM_SYSKEYDOWN):
                    if self._on_key(kb.vkCode):
                        return 1
            return user32.CallNextHookEx(self._hook, nCode, wParam, lParam)

        self._hook_proc = HOOKPROC(_hook_proc)
        self._hook = user32.SetWindowsHookExW(WH_KEYBOARD_LL, self._hook_proc, hmod, 0)
        if not self._hook:
            print(f"[ERROR] hook failed: {k32.GetLastError()}")
            return False
        return True

    def _start_toggle_hotkey(self):
        def poll():
            was_down = False
            while True:
                down = (ctypes.windll.user32.GetAsyncKeyState(VK_RCONTROL) & 0x8000) != 0
                if down and not was_down:
                    was_down = True
                    self._root.after(0, self._toggle)
                elif not down and was_down:
                    was_down = False
                time.sleep(0.03)
        threading.Thread(target=poll, daemon=True).start()

    def _toggle(self):
        self._active = not self._active
        if self._active:
            self._status_var.set("状态: 已激活")
            self._input_str = ""
            self._candidates = []
        else:
            self._status_var.set("状态: 未激活")
            self._hide_candidates()

    def _on_key(self, vk):
        """处理按键，返回 True=吞掉"""
        # 字母
        if 0x41 <= vk <= 0x5A:
            self._input_str += chr(vk).lower()
            self._query()
            self._show_candidates()
            return True
        # 数字 1-9
        if VK_0 + 1 <= vk <= VK_9:
            idx = vk - VK_0 - 1
            if 0 <= idx < len(self._candidates):
                self._selected = idx
                self._commit()
            return True
        # 空格
        if vk == VK_SPACE:
            if self._candidates:
                self._commit()
            return True
        # 回车
        if vk == VK_RETURN:
            if self._input_str:
                self._commit(raw=True)
            return True
        # 退格
        if vk == VK_BACK:
            if self._input_str:
                self._input_str = self._input_str[:-1]
                self._query()
                self._show_candidates()
            return True
        # Esc
        if vk == VK_ESCAPE:
            self._input_str = ""
            self._candidates = []
            self._hide_candidates()
            return True
        return False

    def _query(self):
        if not self._input_str:
            self._candidates = []
            return
        results = self.engine.query(self._input_str, "pinyin")
        self._candidates = [r[0] for r in results[:9]]
        self._selected = 0

    def _show_candidates(self):
        if not self._input_str or not self._candidates:
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
            self._candidate_win.configure(bg="#ffffff")
            self._cand_labels = []
            for i in range(9):
                lbl = tk.Label(self._candidate_win, text="", font=("Microsoft YaHei UI", 12),
                               bg="#ffffff", fg="#5b6b7c", anchor="w", padx=8, pady=2)
                lbl.pack(side=tk.LEFT)
                self._cand_labels.append(lbl)

        for i, lbl in enumerate(self._cand_labels):
            if i < len(self._candidates):
                prefix = f"{i+1}. " if i == self._selected else f"{i+1}  "
                color = "#c47f2a" if i == self._selected else "#5b6b7c"
                lbl.config(text=prefix + self._candidates[i], fg=color)
            else:
                lbl.config(text="", fg="#5b6b7c")

        self._candidate_win.geometry(f"+{x}+{y + 20}")
        self._candidate_win.deiconify()

    def _hide_candidates(self):
        if self._candidate_win:
            self._candidate_win.withdraw()

    def _commit(self, raw=False):
        if raw:
            text = self._input_str
        elif self._candidates and self._selected < len(self._candidates):
            text = self._candidates[self._selected]
            self.engine.learn(self._input_str, text)
        else:
            text = self._input_str

        if text:
            self._send_text(text)

        self._input_str = ""
        self._candidates = []
        self._hide_candidates()

    def _send_text(self, text):
        """使用 SendInput 发送 Unicode 文本"""
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
            _fields_ = [("type", wintypes.DWORD), ("_input", _INPUT)]

        KEYEVENTF_UNICODE = 0x0004
        KEYEVENTF_KEYUP = 0x0002

        inputs = []
        for char in text:
            inp = INPUT()
            inp.type = 1
            inp._input.ki.wVk = 0
            inp._input.ki.wScan = ord(char)
            inp._input.ki.dwFlags = KEYEVENTF_UNICODE
            inp._input.ki.time = 0
            inp._input.ki.dwExtraInfo = None
            inputs.append(inp)

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


def main():
    app = PinyinIME()
    app.start()


if __name__ == "__main__":
    main()
