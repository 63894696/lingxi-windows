#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Windows 端引擎冒烟测试 —— 直接调 prisir_ime.dll 的 C ABI（无 GUI、无假 PASS）。

用法:
  python smoke_win.py                                   # 用 stage 里的 dll + db
  python smoke_win.py --dll x.dll --db y.db
  python smoke_win.py --json out.json                   # 机读结果

检查项:
  1) 经典全拼/简拼/整句（回归基线，改动后必须还过）
  2) 词库抢救结果（key 曾为 '100' 的 98 万行能否命中）
  3) 领域术语（新采的专业名词能否命中）
  4) 查询延迟（内存索引前后是否同量级）
"""
import argparse
import ctypes
import json
import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_DLL = os.path.join(HERE, "..", "installer", "stage", "prisir_ime.dll")
DEFAULT_DB = os.path.join(HERE, "..", "installer", "stage", "models", "ciku.db")

# (输入, 期望出现在候选前 N 个里的词, 说明)
CASES = [
    ("nihao", "你好", "全拼"),
    ("shijie", "世界", "全拼"),
    ("sj", "世界", "简拼（旧实现 segment('sj') 为空 → 完全失效）"),
    ("zhrmghg", "中华人民共和国", "每字单字母整句"),
    ("jintiantianqi", "今天天气", "混拼/整句"),
    ("zhishizhengliu", "知识蒸馏", "领域术语（AI 语料管线产出）"),
    ("duomotai", "多模态", "AI 术语"),
    ("niaoduzhengwanqi", "尿毒症晚期", "词库抢救（原本 key='100' 打不出来）"),
    ("zhongyaoqiepianji", "中药切片机", "词库抢救"),
    ("erjianbanxiazhai", "二尖瓣狭窄", "医学领域术语"),
]

TOP_N = 5


def load(dll, db):
    lib = ctypes.CDLL(dll)
    lib.prisir_ime_load.restype = ctypes.c_void_p
    lib.prisir_ime_load.argtypes = [ctypes.c_char_p, ctypes.c_int]
    lib.prisir_ime_query.restype = ctypes.c_void_p
    lib.prisir_ime_query.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
    lib.prisir_ime_decode.restype = ctypes.c_void_p
    lib.prisir_ime_decode.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
    lib.prisir_ime_free_string.restype = None
    lib.prisir_ime_free_string.argtypes = [ctypes.c_void_p]
    lib.prisir_ime_free.restype = None
    lib.prisir_ime_free.argtypes = [ctypes.c_void_p]

    t0 = time.time()
    h = lib.prisir_ime_load(db.encode(), 0)
    assert h, f"引擎加载失败: dll={dll} db={db}"
    print(f"[load] {time.time() - t0:.2f}s  db={os.path.basename(db)}")
    return lib, h


def take(lib, p):
    if not p:
        return None
    s = ctypes.string_at(p).decode("utf-8", "replace")
    lib.prisir_ime_free_string(p)
    return s


def parse_cands(raw):
    """query 返回 JSON：可能是 [{word,weight}] 或 [word] —— 两种都吃。"""
    if not raw:
        return []
    try:
        data = json.loads(raw)
    except json.JSONDecodeError:
        return [raw]
    out = []
    for x in data if isinstance(data, list) else [data]:
        if isinstance(x, dict):
            out.append(str(x.get("word") or x.get("value") or x.get("text") or ""))
        else:
            out.append(str(x))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dll", default=DEFAULT_DLL)
    ap.add_argument("--db", default=DEFAULT_DB)
    ap.add_argument("--json", default="")
    a = ap.parse_args()

    lib, h = load(a.dll, a.db)
    results, fails = [], 0
    for inp, want, note in CASES:
        t0 = time.time()
        cands = parse_cands(take(lib, lib.prisir_ime_query(h, inp.encode())))
        ms = (time.time() - t0) * 1000
        hit = want in cands[:TOP_N]
        ok = "PASS" if hit else "FAIL"
        if not hit:
            fails += 1
        print(f"[{ok}] {inp:<18} {ms:6.1f}ms  期望 {want:<8} 实得 {cands[:TOP_N]}  ({note})")
        results.append({"input": inp, "want": want, "got": cands[:TOP_N],
                        "hit": hit, "ms": round(ms, 2), "note": note})

    # 整句解码抽查
    for inp, want in (("zhrmghg", "中华人民共和国"), ("jintiantianqi", "今天天气")):
        t0 = time.time()
        s = take(lib, lib.prisir_ime_decode(h, inp.encode(), 3))
        ms = (time.time() - t0) * 1000
        hit = bool(s) and want in str(s)
        if not hit:
            fails += 1
        print(f"[{'PASS' if hit else 'FAIL'}] decode {inp:<14} {ms:6.1f}ms  -> {s}")
        results.append({"input": "decode:" + inp, "want": want, "got": s,
                        "hit": hit, "ms": round(ms, 2), "note": "束搜索整句"})

    lib.prisir_ime_free(h)
    print(f"\n{'=' * 60}\n结果: {len(results) - fails}/{len(results)} 通过")
    if a.json:
        json.dump({"results": results, "fails": fails}, open(a.json, "w", encoding="utf-8"),
                  ensure_ascii=False, indent=2)
        print(f"[out] {a.json}")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
