#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""改版本号 — prisIragent_ime/VERSION.txt 是 Windows 端唯一来源。

用法:
  python installer/set_version.py prisIragent_ime/VERSION.txt 1.0.0-beta.25   # 完整 semver
  python installer/set_version.py prisIragent_ime/VERSION.txt beta.25          # 只给后缀，沿用现有 base

为什么单独一个脚本：版本号散落成三处（NSIS 默认值 / stage/VERSION.txt / 源码 VERSION.txt）
曾导致"打包说是 beta.24、源码还是 beta.9"的错位。现在只改这一处，其余全部生成。
"""
import datetime
import re
import sys


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    path, arg = sys.argv[1], sys.argv[2].strip()
    s = open(path, encoding="utf-8").read()
    m = re.search(r"^PRIMARY_VERSION=(.+)$", s, re.M)
    if not m:
        print(f"[x] {path} 缺 PRIMARY_VERSION")
        return 1
    cur = m.group(1).strip()
    if arg[0].isdigit():
        ver = arg
    else:
        ver = f"{cur.split('-')[0]}-{arg}"
    if not re.fullmatch(r"\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?", ver):
        print(f"[x] 版本号不合法: {ver}")
        return 1
    s = re.sub(r"^PRIMARY_VERSION=.*$", f"PRIMARY_VERSION={ver}", s, flags=re.M)
    s = re.sub(r"^BUILD_DATE=.*$", f"BUILD_DATE={datetime.date.today().isoformat()}", s, flags=re.M)
    open(path, "w", encoding="utf-8", newline="\n").write(s)
    print(f"[build] 版本 {cur} -> {ver}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
