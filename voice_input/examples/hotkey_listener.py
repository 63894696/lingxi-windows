"""voice_input 后台 hotkey 监听(v1.2.4 ship 2026-07-02 21:05)

后台跑这个脚本,按 hotkey 就触发 mic 录音 + ASR + 唤醒词检测 + inject 到当前焦点窗口。

v1.2.4 改进(2026-07-02 21:05):
  - 改用 keyboard 库(更可靠的全局 hotkey,无需管理员权限)
  - 改用 F8 / F9 简单热键(更兼容各种 PowerShell 终端)
  - 加诊断 print 看 hotkey 注册状态
  - cwd 智能解析(任何目录跑都对)

用法(任选一种):
  1. python examples/hotkey_listener.py      # 项目根
  2. python ../examples/hotkey_listener.py   # scripts/ 目录
  3. 双击 scripts/start_hotkey.bat          # Windows 用户友好

热键:
  F8    — 触发 mic 录音 + ASR + 唤醒词 + inject(v1.2.4 默认)
  F9    — 退出 listener
  Ctrl+Alt+R / Ctrl+Alt+Q — 兼容 v1.2.3 旧版(也注册,有些 PowerShell 不能触发就用 F8)

前置(可选 — listener 自动起 daemon):
  1. voice_input 3 daemon 已启动(start_all.py)或自动起
  2. USB 麦克风已连接
  3. 想要 inject 的窗口(如 notepad)先开 + 焦点在前
"""
from __future__ import annotations
import os
import sys
import time
import threading
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
os.chdir(PROJECT_ROOT)
sys.path.insert(0, str(PROJECT_ROOT / "src"))
sys.path.insert(0, str(PROJECT_ROOT / "clients"))

from voice_input.asr_daemon import detect_wake_word, DEFAULT_WAKE_WORDS
from clients.oi_client import VoiceInputClient


class VoiceInputListener:
    def __init__(self):
        self.client = VoiceInputClient()
        self.is_running = True
        self.last_event_time = 0
        self.event_lock = threading.Lock()
        self.debounce_seconds = 1.0
        self.trigger_count = 0

    def trigger_record(self, source="hotkey"):
        """F8 / Ctrl+Alt+R 触发:mic 录音 + ASR + 唤醒词 + inject"""
        with self.event_lock:
            now = time.time()
            if now - self.last_event_time < self.debounce_seconds:
                print(f"  [debounce] 忽略重复触发({now - self.last_event_time:.2f}s)")
                return
            self.last_event_time = now
            self.trigger_count += 1

        print()
        print("=" * 60)
        print(f"  [{source}] 触发 #{self.trigger_count} — 启动 mic 录音")
        print("=" * 60)
        try:
            r = self.client.record_and_transcribe(max_seconds=10, input_device=1)
            text = r.get("text", "")
            duration = r.get("duration_ms", 0)
            print(f"  识别: {text!r} ({duration}ms)")

            if not text:
                print("  ⚠ 识别为空,跳过 inject(可能没说话或 mic 没采到)")
                return

            wake = detect_wake_word(text)
            print(f"  唤醒词: {wake!r}")

            if wake:
                cmd = strip_wake_word_inline(text, wake)
                print(f"  剥掉唤醒词: {cmd!r}")
                r = self.client.inject_text(cmd)
            else:
                print(f"  ⚠ 未检测到 wake word({DEFAULT_WAKE_WORDS}),直接 inject 完整文本")
                r = self.client.inject_text(text)
            print(f"  inject 结果: {r}")
        except Exception as e:
            print(f"  ✗ ERR: {type(e).__name__}: {e}")
        finally:
            print("=" * 60)
            print(f"  等待下次热键触发(按 F9 / Ctrl+Alt+Q 退出)")
            print("=" * 60)
            print()

    def stop(self):
        self.is_running = False
        print()
        print("  [F9/Ctrl+Alt+Q] 按下 — 退出 listener")


def strip_wake_word_inline(text: str, wake_word: str) -> str:
    """从识别文本里去掉 wake_word 子串"""
    if not wake_word:
        return text
    text_clean = text.replace(" ", "").lower()
    wake_clean = wake_word.lower().replace(" ", "")
    if wake_clean not in text_clean:
        return text
    # 找位置 + 切片
    idx = text_clean.find(wake_clean)
    orig_idx = 0
    clean_idx = 0
    while clean_idx < idx and orig_idx < len(text):
        if text[orig_idx].lower() != " ":
            clean_idx += 1
        orig_idx += 1
    end_idx = orig_idx
    clean_idx = 0
    while clean_idx < len(wake_clean) and end_idx < len(text):
        if text[end_idx].lower() != " ":
            clean_idx += 1
        end_idx += 1
    return (text[:orig_idx] + text[end_idx:]).strip() or text


def auto_start_daemon():
    """自动启动 3 daemon"""
    import subprocess
    start_all = PROJECT_ROOT / "scripts" / "start_all.py"
    print(f"  → 调 {start_all}")
    subprocess.Popen(
        [sys.executable, str(start_all)],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )


def main():
    print("=" * 60)
    print(f"voice_input 后台 Hotkey Listener (v1.2.4 ship 2026-07-02 21:05)")
    print(f"  项目根: {PROJECT_ROOT}")
    print("=" * 60)
    print()
    print("  热键:")
    print("    F8               — 触发 mic 录音 + ASR + 唤醒词 + inject")
    print("    F9               — 退出 listener")
    print("    Ctrl+Alt+R       — 兼容 v1.2.3(部分终端可能不响应)")
    print("    Ctrl+Alt+Q       — 兼容 v1.2.3 退出")
    print()

    listener = VoiceInputListener()

    # 健康检查(失败时自动启动 daemon)
    try:
        health = listener.client.health()
        print(f"  [/health] status={health.get('status')}")
    except Exception as e:
        print(f"  ✗ /health 失败:{e},自动启动 start_all.py")
        auto_start_daemon()
        time.sleep(12)
        try:
            health = listener.client.health()
            print(f"  [/health 重试] status={health.get('status')}")
        except Exception as e2:
            print(f"  ✗ 自动启动后还是失败:{e2}")
            return

    # 注册 hotkey(用 keyboard 库,比 pynput 稳)
    try:
        import keyboard
        print()
        print("  注册 hotkey(用 keyboard 库)...")

        def on_f8():
            threading.Thread(target=listener.trigger_record, args=("F8",), daemon=True).start()

        def on_f9():
            listener.stop()
            # 用 os._exit(0) 强制退出(因为 keyboard.wait 阻塞)
            import os
            os._exit(0)

        def on_ctrl_alt_r():
            threading.Thread(target=listener.trigger_record, args=("Ctrl+Alt+R",), daemon=True).start()

        def on_ctrl_alt_q():
            listener.stop()
            import os
            os._exit(0)

        keyboard.add_hotkey("f8", on_f8, suppress=False)
        keyboard.add_hotkey("f9", on_f9, suppress=False)
        keyboard.add_hotkey("ctrl+alt+r", on_ctrl_alt_r, suppress=False)
        keyboard.add_hotkey("ctrl+alt+q", on_ctrl_alt_q, suppress=False)
        print("    ✓ F8         (trigger mic)")
        print("    ✓ F9         (exit listener)")
        print("    ✓ Ctrl+Alt+R (trigger mic, 兼容)")
        print("    ✓ Ctrl+Alt+Q (exit listener, 兼容)")
    except ImportError:
        print("  ✗ keyboard 库未装,跑: pip install keyboard")
        return
    except Exception as e:
        print(f"  ✗ 注册 hotkey 失败:{e}")
        print(f"  (提示:可能需要管理员权限,在 PowerShell 管理员窗口跑)")
        return

    print()
    print("  等待热键触发...  (按 F8 测一下)")
    print()

    # 阻塞等待 hotkey
    try:
        import keyboard
        keyboard.wait()  # 永久阻塞直到 F9 / Ctrl+Alt+Q 触发 os._exit(0)
    except KeyboardInterrupt:
        print()
        print("  KeyboardInterrupt 退出")


if __name__ == "__main__":
    main()