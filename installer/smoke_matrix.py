#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""三端 ABI 冒烟矩阵 —— 同一份用例跑 Windows DLL / Linux .so /（将来）Android JNI。

为什么需要：引擎同源是三端唯一的技术优势，但"同源"没人验就等于没有。
每次改引擎，Windows 用 stage 的 dll、Linux 用 libprisir_ime.so、Android 用 JNI 各跑一遍
`prisir_ime/abi_cases.json`，三端结果必须一致。

用法：
  python installer/smoke_matrix.py                                   # Windows stage 默认路径
  python installer/smoke_matrix.py --lib x.dll --db y.db
  python installer/smoke_matrix.py --lib ./libprisir_ime.so --db ./ciku.db    # Linux
  python installer/smoke_matrix.py --json out.json

退出码 0 = 全通过；1 = 有失败（可接 CI / 打包前置检查）。
"""
import argparse
import ctypes
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
DEFAULT_LIB = os.path.join(HERE, "stage", "prisir_ime.dll")
DEFAULT_DB = os.path.join(HERE, "stage", "models", "ciku.db")
CASES = os.path.join(ROOT, "prisir_ime", "abi_cases.json")


def bind(lib_path):
    lib = ctypes.CDLL(lib_path)
    vp = ctypes.c_void_p
    lib.prisir_ime_load.restype = vp
    lib.prisir_ime_load.argtypes = [ctypes.c_char_p, ctypes.c_int]
    lib.prisir_ime_free.restype = None
    lib.prisir_ime_free.argtypes = [vp]
    lib.prisir_ime_free_string.restype = None
    lib.prisir_ime_free_string.argtypes = [vp]
    lib.prisir_ime_query.restype = vp
    lib.prisir_ime_query.argtypes = [vp, ctypes.c_char_p]
    lib.prisir_ime_decode.restype = vp
    lib.prisir_ime_decode.argtypes = [vp, ctypes.c_char_p, ctypes.c_int]
    lib.prisir_ime_classify.restype = ctypes.c_int
    lib.prisir_ime_classify.argtypes = [vp, ctypes.c_char_p]
    lib.prisir_ime_punct.restype = vp
    lib.prisir_ime_punct.argtypes = [vp, ctypes.c_uint, ctypes.c_int]
    lib.prisir_ime_punct_reset.restype = None
    lib.prisir_ime_punct_reset.argtypes = [vp]
    if hasattr(lib, "prisir_ime_abi_version"):
        lib.prisir_ime_abi_version.restype = ctypes.c_int
    return lib


def take(lib, ptr):
    if not ptr:
        return ""
    s = ctypes.string_at(ptr).decode("utf-8", "replace")
    lib.prisir_ime_free_string(ptr)
    return s


def words_of(raw):
    try:
        data = json.loads(raw) if raw else []
    except json.JSONDecodeError:
        return []
    out = []
    for x in data if isinstance(data, list) else [data]:
        if isinstance(x, dict):
            # query 返回 {"word":..}；decode 返回 {"nwords":..,"score":..,"sentence":..}
            out.append(str(x.get("word") or x.get("value") or x.get("sentence") or ""))
        else:
            out.append(str(x))
    return out


def run_case(lib, h, c):
    kind = c["kind"]
    if kind == "query":
        ws = words_of(take(lib, lib.prisir_ime_query(h, c["in"].encode())))
        if "first" in c:
            ok = bool(ws) and ws[0] == c["first"]
            return ok, ws[:5]
        if "contains" in c:
            return c["contains"] in ws, ws[:5]
        if "contains_any" in c:
            return any(x in ws for x in c["contains_any"]), ws[-5:]
        return False, ws[:5]
    if kind == "decode":
        ws = words_of(take(lib, lib.prisir_ime_decode(h, c["in"].encode(), 3)))
        if "first_sentence" in c:
            return bool(ws) and ws[0] == c["first_sentence"], ws[:3]
        joined = "".join(ws)
        return c.get("contains", "") in joined, ws[:3]
    if kind == "classify":
        got = lib.prisir_ime_classify(h, c["in"].encode())
        return got == c["expect"], got
    if kind == "punct_roundtrip":
        lib.prisir_ime_punct_reset(h)
        out = []
        for ch in c["in"]:
            out.append(take(lib, lib.prisir_ime_punct(h, ord(ch), 0)))
        got = "".join(out)
        lib.prisir_ime_punct_reset(h)
        return got == c["expect"], got
    return False, f"未知用例类型 {kind}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--lib", default=DEFAULT_LIB)
    ap.add_argument("--db", default=DEFAULT_DB)
    ap.add_argument("--cases", default=CASES)
    ap.add_argument("--json", default="")
    a = ap.parse_args()

    spec = json.load(open(a.cases, encoding="utf-8"))
    lib = bind(a.lib)
    abi = lib.prisir_ime_abi_version() if hasattr(lib, "prisir_ime_abi_version") else None
    print(f"[matrix] lib={os.path.basename(a.lib)} db={os.path.basename(a.db)} "
          f"abi={abi} 期望={spec.get('abi')}")
    if abi is not None and spec.get("abi") is not None and abi != spec["abi"]:
        print(f"[!] ABI 版本不一致（{abi} != {spec['abi']}）：外壳/引擎不匹配")

    h = lib.prisir_ime_load(a.db.encode(), 0)
    if not h:
        print("[x] 引擎加载失败")
        return 1

    results, fails = [], 0
    for c in spec["cases"]:
        try:
            ok, got = run_case(lib, h, c)
        except Exception as e:  # noqa: BLE001
            ok, got = False, f"异常: {e}"
        fails += 0 if ok else 1
        print(f"[{'PASS' if ok else 'FAIL'}] {c['kind']:<16} {c['in']:<16} → {got}  ({c.get('note','')})")
        results.append({"case": c, "ok": ok, "got": got})

    lib.prisir_ime_free(h)
    n = len(results)
    print(f"\n{'=' * 62}\n矩阵结果: {n - fails}/{n} 通过")
    if a.json:
        json.dump({"lib": a.lib, "abi": abi, "fails": fails, "results": results},
                  open(a.json, "w", encoding="utf-8"), ensure_ascii=False, indent=2)
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
