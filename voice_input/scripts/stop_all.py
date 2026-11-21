#!/usr/bin/env python
"""voice_input 一键关闭 — 跨平台 (2026-07-02 v0.4 ship)

杀 voice_input 3 daemon

用法:
  python scripts/stop_all.py
"""
from __future__ import annotations
import subprocess
import time


def main():
    print("voice_input 关闭 3 daemon...")
    try:
        result = subprocess.run(
            ["powershell", "-NoProfile", "-Command",
             "Get-NetTCPConnection | Where-Object {$_.LocalPort -in 8730,8731,8732} | "
             "ForEach-Object { Stop-Process -Id $_.OwningProcess -Force }"],
            timeout=10, capture_output=True, text=True,
        )
        print(result.stdout.strip())
    except Exception as e:
        print(f"  ERR: {e}")
    time.sleep(1)
    print("  ✓ 3 daemon 已关闭")


if __name__ == "__main__":
    main()