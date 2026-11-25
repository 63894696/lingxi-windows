#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""断言外壳 DLL 已打进新关于文案（防打旧包）。

用法: python installer/check_about_strings.py <prisiragent_ime.dll|prisir_ime_tsf.dll>
"""
from __future__ import annotations

import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    p = Path(sys.argv[1])
    if not p.is_file():
        print(f"[x] 文件不存在: {p}")
        return 1
    b = p.read_bytes()

    def has(s: str) -> bool:
        return s.encode("utf-8") in b or s.encode("utf-16-le") in b

    required = [
        "AGPL-3.0",
        "隐私政策",
        "开放 API",
        "问题反馈",
        "使用说明",
        "GNU Affero General Public License",
    ]
    forbidden = [
        "lsjdlijie",
        "OIE-PCS",
        "反馈联系",
        "隐私说明",
        "匿名更新检查",
    ]
    bad = False
    for s in required:
        ok = has(s)
        print(f"  [{'✓' if ok else 'x'}] require {s!r}")
        if not ok:
            bad = True
    for s in forbidden:
        hit = has(s)
        print(f"  [{'✓' if not hit else 'x'}] forbid  {s!r}")
        if hit:
            bad = True
    if bad:
        print(f"[x] {p.name} 文案断言失败")
        return 1
    print(f"[✓] {p.name} 关于文案断言通过")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
