#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""用户数据落盘位置测试 —— 装到只读目录（Program Files）时学习/黑白名单还能不能用。

背景（2026-11-17 发现）：NSIS 默认装到 `C:\\Program Files\\PrisirIME`，
该目录 `BUILTIN\\Users = ReadAndExecute`，普通用户**不可写**。
老实现把 `ciku.user.db` 放在主库旁边 → 创建失败 → `user_conn=None`
→ **频率学习静默失效**（用户最在意的"常用词自动靠前"没了），黑白名单也写不进去。

本测试模拟"主库目录不可写"：
  1. 把词库复制到一个**只读**目录（Windows 上用 icacls 去掉写权限；失败则跳过该断言）
  2. 设 `PRISIR_IME_DATA` 指向可写目录
  3. 断言：learn 生效、user_list 读得到、黑名单可增删、主库旁边**没有**生成 user.db

用法:
  python smoke_userdata.py
  python smoke_userdata.py --keep     # 保留临时目录
"""
import argparse
import ctypes
import json
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
DLL = os.path.join(HERE, "..", "installer", "stage", "prisir_ime.dll")
DB = os.path.join(HERE, "..", "installer", "stage", "models", "ciku.db")


def bind(dll_path):
    lib = ctypes.CDLL(dll_path)
    lib.prisir_ime_load.restype = ctypes.c_void_p
    lib.prisir_ime_load.argtypes = [ctypes.c_char_p, ctypes.c_int]
    lib.prisir_ime_free.restype = None
    lib.prisir_ime_free.argtypes = [ctypes.c_void_p]
    lib.prisir_ime_free_string.restype = None
    lib.prisir_ime_free_string.argtypes = [ctypes.c_void_p]
    lib.prisir_ime_learn.restype = None
    lib.prisir_ime_learn.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p]
    lib.prisir_ime_user_list.restype = ctypes.c_void_p
    lib.prisir_ime_user_list.argtypes = [ctypes.c_void_p]
    lib.prisir_ime_add_blacklist.restype = ctypes.c_int
    lib.prisir_ime_add_blacklist.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p]
    lib.prisir_ime_get_blacklist.restype = ctypes.c_void_p
    lib.prisir_ime_get_blacklist.argtypes = [ctypes.c_void_p]
    lib.prisir_ime_remove_blacklist.restype = ctypes.c_int
    lib.prisir_ime_remove_blacklist.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
    # 词库窗「保存/删除」在只读安装下的退化路径
    lib.prisir_ime_main_upsert.restype = ctypes.c_int
    lib.prisir_ime_main_upsert.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p, ctypes.c_longlong]
    lib.prisir_ime_main_delete.restype = ctypes.c_int
    lib.prisir_ime_main_delete.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p]
    lib.prisir_ime_query.restype = ctypes.c_void_p
    lib.prisir_ime_query.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
    return lib


def take(lib, p):
    if not p:
        return ""
    s = ctypes.string_at(p).decode("utf-8", "replace")
    lib.prisir_ime_free_string(p)
    return s


def make_readonly(path):
    """Windows: icacls 去掉当前用户写权限。返回是否成功。"""
    user = os.environ.get("USERNAME", "")
    try:
        subprocess.run(["icacls", path, "/deny", f"{user}:(W,AD,WD)"],
                       check=True, capture_output=True)
        return True
    except Exception:  # noqa: BLE001
        return False


def restore(path):
    user = os.environ.get("USERNAME", "")
    subprocess.run(["icacls", path, "/remove:d", user], capture_output=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()

    root = tempfile.mkdtemp(prefix="lingxi_ud_")
    ro_dir = os.path.join(root, "readonly")
    data_dir = os.path.join(root, "userdata")
    os.makedirs(ro_dir)
    os.makedirs(data_dir)
    db = os.path.join(ro_dir, "ciku.db")
    shutil.copy2(DB, db)

    fails = 0

    def check(cond, msg):
        nonlocal fails
        print(("[PASS] " if cond else "[FAIL] ") + msg)
        if not cond:
            fails += 1

    readonly = make_readonly(ro_dir)
    print(f"[setup] 主库目录 {ro_dir} 只读={readonly}；用户数据目录 {data_dir}")

    # 关键：让引擎把用户数据写到 data_dir（模拟 %LOCALAPPDATA%）
    os.environ["PRISIR_IME_DATA"] = data_dir
    lib = bind(DLL)
    h = lib.prisir_ime_load(db.encode(), 0)
    check(bool(h), "引擎加载成功（主库只读也能加载）")
    if not h:
        return 1

    # 1) 学习：写 user.db
    lib.prisir_ime_learn(h, b"banquan", "版权".encode())
    raw = take(lib, lib.prisir_ime_user_list(h))
    words = []
    try:
        words = [x.get("value") for x in json.loads(raw)]
    except Exception:  # noqa: BLE001
        pass
    check("版权" in words, f"learn 生效（user_list 含「版权」）→ {words[:5]}")

    # 2) 用户数据落在 PRISIR_IME_DATA，而不是主库旁边
    ud = os.path.join(data_dir, "ciku.user.db")
    check(os.path.exists(ud), f"user.db 落在用户数据目录: {ud}")
    check(not os.path.exists(os.path.join(ro_dir, "ciku.user.db")),
          "主库旁边没有生成 user.db（只读目录不该被写）")

    # 3) 黑名单：写 user.db，可增可删
    n = lib.prisir_ime_add_blacklist(h, "版权".encode(), b"smoke")
    check(n == 1, f"黑名单写入成功（返回 {n}）")
    listed = []
    try:
        listed = [x["value"] for x in json.loads(take(lib, lib.prisir_ime_get_blacklist(h)))]
    except Exception:  # noqa: BLE001
        pass
    check("版权" in listed, f"get_blacklist 读得到 → {listed[:5]}")
    r = lib.prisir_ime_remove_blacklist(h, "版权".encode())
    check(r == 1, f"黑名单删除成功（返回 {r}）")

    # 3.5) 词库窗「保存 / 删除」在只读安装下也要能用（退化到用户层）
    def cand_words(hh, pinyin):
        raw = take(lib, lib.prisir_ime_query(hh, pinyin.encode()))
        try:
            return [x.get("word") or x.get("value") or "" for x in json.loads(raw)]
        except Exception:  # noqa: BLE001
            return []

    NEW_WORD, NEW_PINYIN = "工具栏词", "gongjulanci"
    up = lib.prisir_ime_main_upsert(h, NEW_PINYIN.encode(), NEW_WORD.encode(), 5000)
    check(up == 1, f"只读安装下「保存」成功（返回 {up}）")
    check(NEW_WORD in cand_words(h, NEW_PINYIN), f"保存后能打出来 → {cand_words(h, NEW_PINYIN)[:3]}")
    dl = lib.prisir_ime_main_delete(h, NEW_PINYIN.encode(), NEW_WORD.encode())
    check(dl >= 1, f"只读安装下「删除」成功（返回 {dl}）")
    check(NEW_WORD not in cand_words(h, NEW_PINYIN), f"删除后候选里没有了 → {cand_words(h, NEW_PINYIN)[:3]}")
    up2 = lib.prisir_ime_main_upsert(h, NEW_PINYIN.encode(), NEW_WORD.encode(), 5000)
    check(up2 == 1 and NEW_WORD in cand_words(h, NEW_PINYIN),
          "再次保存即恢复（删除=可逆的用户层屏蔽）")

    lib.prisir_ime_free(h)

    # 4) 老位置（主库同目录）的 user.db 要能自动搬过来
    #    否则升级后用户之前学过的几千条词全部"消失"，感知就是"越用越不准了"。
    import sqlite3
    legacy_root = os.path.join(root, "legacy")
    legacy_ro = os.path.join(legacy_root, "ro")
    legacy_data = os.path.join(legacy_root, "data")
    os.makedirs(legacy_ro)
    os.makedirs(legacy_data)
    legacy_main = os.path.join(legacy_ro, "ciku.db")
    shutil.copy2(DB, legacy_main)
    legacy_user = os.path.join(legacy_ro, "ciku.user.db")
    lc = sqlite3.connect(legacy_user)
    lc.execute("CREATE TABLE user_word(key TEXT,value TEXT,weight INTEGER,seq INTEGER,PRIMARY KEY(key,value))")
    lc.execute("INSERT INTO user_word VALUES('fupan','复盘',999999,1)")
    lc.commit()
    lc.close()
    os.environ["PRISIR_IME_DATA"] = legacy_data
    h2 = lib.prisir_ime_load(legacy_main.encode(), 0)
    check(bool(h2), "老 user.db 场景：引擎加载成功")
    if h2:
        raw2 = take(lib, lib.prisir_ime_user_list(h2))
        words2 = []
        try:
            words2 = [x.get("value") for x in json.loads(raw2)]
        except Exception:  # noqa: BLE001
            pass
        check("复盘" in words2, f"老 user.db 已迁移（user_list 含「复盘」）→ {words2[:5]}")
        lib.prisir_ime_free(h2)

    if readonly:
        restore(ro_dir)
    if not a.keep:
        shutil.rmtree(root, ignore_errors=True)
    else:
        print(f"[keep] {root}")
    print(f"\n{'=' * 60}\n结果: {'全通过' if fails == 0 else f'{fails} 项失败'}")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
