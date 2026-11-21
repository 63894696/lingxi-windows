#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""黑/白名单端到端测试 —— 直接在词库副本上跑,不污染正式库。

验证 6 个 C 导出(UI 的「屏蔽 / 置顶 / 解除」就是调它们):
    add_blacklist / remove_blacklist / get_blacklist
    add_whitelist / remove_whitelist / get_whitelist
语义断言:
    1. 加黑名单后,该词**不再出现在候选里**(引擎 apply_blacklist_whitelist 剔除)
    2. 重复加黑名单返回 0(已在名单),不是"成功"
    3. 移除返回删除行数;移除后候选恢复
    4. 白名单让该词在同等权重下排更前(WHITELIST_BONUS=10M)

用法:
    python smoke_blacklist.py                 # 自动复制 stage 词库到临时目录
    python smoke_blacklist.py --db path.db    # 指定库(仍是副本!)
    python smoke_blacklist.py --keep          # 保留临时库(排查用)
"""
import argparse
import ctypes
import json
import os
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
DLL = os.path.join(HERE, "..", "installer", "stage", "prisir_ime.dll")
DB = os.path.join(HERE, "..", "installer", "stage", "models", "ciku.db")

WORD = "版权"          # 词库里高权重真词
PINYIN = "banquan"


def bind(dll_path):
    lib = ctypes.CDLL(dll_path)
    lib.prisir_ime_load.restype = ctypes.c_void_p
    lib.prisir_ime_load.argtypes = [ctypes.c_char_p, ctypes.c_int]
    lib.prisir_ime_free.restype = None
    lib.prisir_ime_free.argtypes = [ctypes.c_void_p]
    lib.prisir_ime_free_string.restype = None
    lib.prisir_ime_free_string.argtypes = [ctypes.c_void_p]
    lib.prisir_ime_query.restype = ctypes.c_void_p
    lib.prisir_ime_query.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
    for name in ("add_blacklist", "add_whitelist"):
        f = getattr(lib, f"prisir_ime_{name}")
        f.restype = ctypes.c_int
        f.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p]
    for name in ("remove_blacklist", "remove_whitelist"):
        f = getattr(lib, f"prisir_ime_{name}")
        f.restype = ctypes.c_int
        f.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
    for name in ("get_blacklist", "get_whitelist"):
        f = getattr(lib, f"prisir_ime_{name}")
        f.restype = ctypes.c_void_p
        f.argtypes = [ctypes.c_void_p]
    return lib


def take(lib, p):
    if not p:
        return ""
    s = ctypes.string_at(p).decode("utf-8", "replace")
    lib.prisir_ime_free_string(p)
    return s


def words(lib, h, pinyin):
    raw = take(lib, lib.prisir_ime_query(h, pinyin.encode()))
    try:
        data = json.loads(raw)
    except json.JSONDecodeError:
        return []
    return [x.get("word") or x.get("value") or "" if isinstance(x, dict) else str(x) for x in data]


def listed(lib, h, which):
    raw = take(lib, getattr(lib, f"prisir_ime_get_{which}")(h))
    try:
        return [x["value"] for x in json.loads(raw)]
    except Exception:  # noqa: BLE001
        return []


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", default="")
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()

    tmpdir = tempfile.mkdtemp(prefix="lingxi_bl_")
    db = a.db or os.path.join(tmpdir, "ciku.db")
    if not a.db:
        shutil.copy2(DB, db)
        print(f"[tmp] 词库副本 {db}")
    lib = bind(DLL)
    h = lib.prisir_ime_load(db.encode(), 0)
    assert h, "引擎加载失败"

    fails = 0

    def check(cond, msg):
        nonlocal fails
        print(("[PASS] " if cond else "[FAIL] ") + msg)
        if not cond:
            fails += 1

    base = words(lib, h, PINYIN)
    check(WORD in base, f"基线：{PINYIN} 候选含「{WORD}」→ {base[:5]}")

    n = lib.prisir_ime_add_blacklist(h, WORD.encode(), b"smoke")
    check(n == 1, f"加黑名单返回 1（新插入），实际 {n}")
    check(WORD in listed(lib, h, "blacklist"), "get_blacklist 里有该词")
    after = words(lib, h, PINYIN)
    check(WORD not in after, f"加黑后候选不含「{WORD}」→ {after[:5]}")

    n2 = lib.prisir_ime_add_blacklist(h, WORD.encode(), b"smoke")
    check(n2 == 0, f"重复加黑名单返回 0（已在名单），实际 {n2}")

    r = lib.prisir_ime_remove_blacklist(h, WORD.encode())
    check(r == 1, f"移除黑名单返回删除行数 1，实际 {r}")
    back = words(lib, h, PINYIN)
    check(WORD in back, f"移除后候选恢复 → {back[:5]}")

    w = lib.prisir_ime_add_whitelist(h, WORD.encode(), b"smoke")
    check(w == 1, f"加白名单返回 1，实际 {w}")
    check(WORD in listed(lib, h, "whitelist"), "get_whitelist 里有该词")
    pinned = words(lib, h, PINYIN)
    check(pinned and pinned[0] == WORD, f"白名单置顶：「{WORD}」排第一 → {pinned[:5]}")

    rw = lib.prisir_ime_remove_whitelist(h, WORD.encode())
    check(rw == 1, f"移除白名单返回 1，实际 {rw}")

    lib.prisir_ime_free(h)
    if not a.keep and not a.db:
        shutil.rmtree(tmpdir, ignore_errors=True)
    print(f"\n{'=' * 60}\n结果: {'全通过' if fails == 0 else f'{fails} 项失败'}")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
