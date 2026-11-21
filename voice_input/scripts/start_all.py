#!/usr/bin/env python
"""voice_input 一键启动 — 跨平台 (2026-07-02 v0.4 ship)

启动 3 daemon: ASR (8731) + TTS (8732) + Orchestrator (8730)
后台运行,日志写到 ~/.voice_input/logs/

用法(Windows):
  python scripts/start_all.py

用法(用户友好):
  双击 scripts\start_all.bat
"""
from __future__ import annotations
import os
import sys
import time
import subprocess
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
LOG_DIR = Path.home() / ".voice_input" / "logs"
LOG_DIR.mkdir(parents=True, exist_ok=True)


def kill_existing():
    """先杀已存在的 voice_input 3 daemon"""
    try:
        subprocess.run(
            ["powershell", "-NoProfile", "-Command",
             "Get-NetTCPConnection | Where-Object {$_.LocalPort -in 8730,8731,8732} | "
             "ForEach-Object { Stop-Process -Id $_.OwningProcess -Force }"],
            timeout=10, capture_output=True,
        )
    except Exception:
        pass
    time.sleep(1)


def start_daemon(name: str, module: str) -> subprocess.Popen:
    """后台启动 1 个 daemon(继承当前 env + PYTHONPATH)"""
    env = {**os.environ, "PYTHONPATH": str(PROJECT_ROOT / "src")}
    log_file = open(LOG_DIR / f"{name}.log", "a", encoding="utf-8")
    p = subprocess.Popen(
        [sys.executable, "-m", module],
        cwd=str(PROJECT_ROOT), env=env,
        stdout=log_file, stderr=subprocess.STDOUT,
    )
    return p


def main():
    print("=" * 60)
    print("voice_input 启动 3 daemon (项目根:", PROJECT_ROOT, ")")
    print("=" * 60)

    print("\n[0] 杀掉旧 daemon...")
    kill_existing()

    print("\n[1] 启动 3 daemon...")
    procs = {
        "asr": start_daemon("asr", "voice_input.asr_daemon"),
        "tts": start_daemon("tts", "voice_input.tts_daemon"),
        "orch": start_daemon("orch", "voice_input.orchestrator"),
    }
    for name, p in procs.items():
        print(f"  ✓ {name} daemon PID={p.pid}")

    print(f"\n  等待 10s 让 onnx 模型加载...")
    time.sleep(10)

    # 简单健康检查
    import httpx
    try:
        r = httpx.get("http://127.0.0.1:8730/health", timeout=3.0)
        print(f"  ✓ /health status={r.json().get('status')}")
    except Exception as e:
        print(f"  ⚠ /health 失败:{e}")

    print("\n" + "=" * 60)
    print("voice_input 3 daemon 全部起来!")
    print(f"  日志: {LOG_DIR}")
    print(f"  关 daemon: python scripts/stop_all.py")
    print(f"  跑 e2e 验证: python scripts/verify_offline.py")
    print("=" * 60)


if __name__ == "__main__":
    main()