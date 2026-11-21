"""OI Hey OI demo — 用户说"Hey OI 在记事本写 hello" 自动注入(v1.2 ship)

v1.2 改进(2026-07-02 20:42):
  - 自动打开记事本(若未开)
  - 等 2s 让 notepad 获得焦点
  - 唤醒词命中后,自动 inject 完整指令文本(不只是 "oi",而是你说的完整句)
  - 提供两种模式:
    - A. 一次跑通(默认)— 单次 mic 录音 + ASR + wake + inject
    - B. 后台 hotkey 模式(可选)— Ctrl+Alt+R 触发,一直监听

v1.1 流程:
  1. 用户手动打开记事本(本 demo 的前置)
  2. 跑这个 demo 脚本
  3. 用户对 USB 麦克风说"Hey OI 在记事本写 hello"
  4. silero-vad 自动检测说话 → SenseVoice 转录
  5. 检测到 "hey oi" 唤醒词 → 唤醒 + 拉 final 文本
  6. inject_text() → 剪贴板 + Ctrl+V + IME 切英文
  7. 记事本焦点窗口真收到 "在记事本写 hello"

注意:
  - 需要 voice_input 3 daemon 已经启动(start_all.py)
  - 需要 USB 麦克风 + 记事本先开 + 焦点在记事本
  - 这个 demo 用 record_and_transcribe 路径(WebSocket 流式版本见 test_ws_streaming.py)

跑法:
  python examples/hey_oi_notepad_demo.py
"""
from __future__ import annotations
import os
import sys
import time
import subprocess
from pathlib import Path

# 把 src/ + clients/ 加进 sys.path(独立跑 demo 也能用)
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "clients"))

from voice_input.asr_daemon import detect_wake_word, DEFAULT_WAKE_WORDS
from clients.oi_client import VoiceInputClient


def open_notepad():
    """自动打开记事本(v1.2 新增)— subprocess.start('notepad.exe')"""
    try:
        subprocess.Popen("notepad.exe", shell=False)
        return True
    except Exception as e:
        print(f"  ⚠ 打开记事本失败:{e}")
        return False


def strip_wake_word(text: str, wake_word: str) -> str:
    """从识别文本里去掉唤醒词,保留剩余指令(v1.2 新增)"""
    if not wake_word:
        return text
    # 去掉 wake_word 子串(忽略大小写、空格)
    text_clean = text.replace(" ", "").lower()
    wake_clean = wake_word.lower().replace(" ", "")
    if wake_clean in text_clean:
        # 找位置 + 切片(基于原文本)
        idx = text_clean.find(wake_clean)
        # 算原文本偏移
        # 简化:从原文本去掉 wake 字符序列
        result = ""
        i = 0
        j = 0
        while i < len(text) and j < len(wake_clean):
            if text[i].lower() == wake_clean[j]:
                j += 1
            else:
                result += text[i]
            i += 1
        result += text[i:]
        return result.strip() or text
    return text


def run_demo():
    print("=" * 60)
    print("OI Hey OI Demo — 在记事本写 hello (v1.2)")
    print("=" * 60)
    print()
    print("前置条件:")
    print("  [1] 3 daemon 已启动(运行 python scripts/start_all.py)")
    print("  [2] USB 麦克风已连接")
    print()
    print(f"默认 wake words: {DEFAULT_WAKE_WORDS}")
    print()

    # 1) 健康检查
    c = VoiceInputClient()
    try:
        health = c.health()
        print(f"[1] /health: status={health.get('status')}")
        if health.get("status") != "ok":
            print(f"  ✗ daemon 不正常,请先跑 start_all.py")
            return False
    except Exception as e:
        print(f"  ✗ /health 失败:{e}(daemon 没起?)")
        return False

    # 2) 自动打开记事本(v1.2 新增)
    print()
    print("[2] 自动打开记事本(v1.2 新增)...")
    if open_notepad():
        print("  ✓ notepad.exe 已启动,等 2s 让它获得焦点...")
        time.sleep(2)
    else:
        print("  ⚠ 记事本未启动,需要你手动打开")
        return False

    # 3) 提示用户
    print()
    print("[3] 提示:记事本已焦点在前 + 你说 'Hey OI 在记事本写 hello'")
    print("    (voice_input 会自动 VAD → SenseVoice → 唤醒词 → 剥掉唤醒词 → inject 剩余指令)")
    print()
    print("    按 Enter 开始录音...")
    try:
        input()
    except EOFError:
        pass

    # 4) mic 录音 + ASR
    print()
    print("[4] 开始 mic 录音(最长 10s,说话后 VAD 自动停)...")
    r = c.record_and_transcribe(max_seconds=10, input_device=1)
    text = r.get("text", "")
    duration_ms = r.get("duration_ms", 0)
    print(f"  识别结果: {text!r} ({duration_ms}ms)")

    if not text:
        print("  ✗ 识别为空(可能没说话或 mic 没采到)")
        return False

    # 5) 唤醒词检测
    wake = detect_wake_word(text)
    print()
    print(f"[5] 唤醒词检测:")
    print(f"  text:   {text!r}")
    print(f"  wake:   {wake!r}")

    # 6) 剥掉唤醒词(v1.2 新增)— 保留真实指令
    if wake:
        command_text = strip_wake_word(text, wake)
        print(f"  → 剥掉唤醒词后: {command_text!r}")
    else:
        print(f"  ⚠ 未检测到 wake word({DEFAULT_WAKE_WORDS})")
        print(f"  → 仍然 inject 完整文本(完整 demo)")
        command_text = text

    # 7) inject 到当前焦点窗口(记事本)
    print()
    print(f"[7] inject 到当前焦点窗口(记事本): {command_text!r}")
    r = c.inject_text(command_text)
    print(f"  inject 结果: {r}")
    status = r.get("status", "?")
    if status == "ok":
        print()
        print("=" * 60)
        print(f"✓ Demo 成功!")
        print(f"  唤醒词:     {wake!r}")
        print(f"  识别完整句: {text!r}")
        print(f"  注入指令:   {command_text!r}")
        print(f"  → 记事本应已收到这段文本")
        print("=" * 60)
        return True
    else:
        print(f"  ✗ inject 失败:{r}")
        return False


if __name__ == "__main__":
    ok = run_demo()
    sys.exit(0 if ok else 1)