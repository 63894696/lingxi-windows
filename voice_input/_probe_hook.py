# -*- coding: utf-8 -*-
"""探针:验证 WH_KEYBOARD_LL 钩子是否只吞右 Alt keyup,不影响其他键"""
import ctypes
import ctypes.wintypes as wt
import time
import sys

WH_KEYBOARD_LL = 13
WM_KEYUP = 0x0101
WM_SYSKEYUP = 0x0105
VK_RMENU = 0xA5

user32 = ctypes.windll.user32
k32 = ctypes.windll.kernel32

class KBDLLHOOKSTRUCT(ctypes.Structure):
    _fields_ = [
        ("vkCode", wt.DWORD),
        ("scanCode", wt.DWORD),
        ("flags", wt.DWORD),
        ("time", wt.DWORD),
        ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong)),
    ]

HOOKPROC = ctypes.WINFUNCTYPE(ctypes.c_long, ctypes.c_int, wt.WPARAM, wt.LPARAM)

user32.SetWindowsHookExW.restype = wt.HHOOK
user32.SetWindowsHookExW.argtypes = [ctypes.c_int, HOOKPROC, wt.HMODULE, wt.DWORD]
user32.CallNextHookEx.restype = ctypes.c_long
user32.CallNextHookEx.argtypes = [wt.HHOOK, ctypes.c_int, wt.WPARAM, wt.LPARAM]
k32.GetModuleHandleW.restype = wt.HMODULE

hook = None
hook_proc = None

def _hook_proc(nCode, wParam, lParam):
    if nCode == 0:
        kb = ctypes.cast(lParam, ctypes.POINTER(KBDLLHOOKSTRUCT)).contents
        # 只吞右 Alt keyup
        if kb.vkCode == VK_RMENU and wParam in (WM_KEYUP, WM_SYSKEYUP):
            print(f"[HOOK] suppress RAlt keyup vk={kb.vkCode} wParam={wParam:#x}")
            return 1
        # 打印其他键,确认没误吞
        if kb.vkCode not in (VK_RMENU,):
            print(f"[HOOK] pass vk={kb.vkCode:#x} wParam={wParam:#x}")
    return user32.CallNextHookEx(hook, nCode, wParam, lParam)

def install():
    global hook, hook_proc
    hook_proc = HOOKPROC(_hook_proc)
    hmod = k32.GetModuleHandleW(None)
    hook = user32.SetWindowsHookExW(WH_KEYBOARD_LL, hook_proc, hmod, 0)
    if not hook:
        print(f"[ERR] SetWindowsHookExW failed, GetLastError={k32.GetLastError()}")
        return False
    print("[OK] hook installed")
    return True

def uninstall():
    global hook
    if hook:
        user32.UnhookWindowsHookEx(hook)
        hook = None
        print("[OK] hook uninstalled")

if __name__ == "__main__":
    if not install():
        sys.exit(1)
    print("请按几个键(包括右 Alt),观察输出。10 秒后自动退出...")
    # 必须有个消息循环,否则低级钩子收不到
    import tkinter as tk
    root = tk.Tk()
    root.withdraw()
    root.after(10000, root.destroy)
    root.mainloop()
    uninstall()
