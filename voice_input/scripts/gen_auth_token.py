#!/usr/bin/env python
"""生成 voice_input auth_token(2026-07-02 ship 骨架)

用法:
  python scripts/gen_auth_token.py
"""
from __future__ import annotations
import sys
from pathlib import Path

# 加 src/ 到 sys.path
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from voice_input.auth import generate_auth_token


if __name__ == "__main__":
    token = generate_auth_token()
    print(f"✓ auth_token 已生成")
    print(f"  路径: ~/.voice_input/auth_token")
    print(f"  长度: {len(token)} chars")
    print(f"  值: {token[:16]}...(后 48 字符省略,保密)")