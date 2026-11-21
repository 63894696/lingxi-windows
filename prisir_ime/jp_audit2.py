"""词库体检 v2：带**字符数守卫**的 jp 修复。

v1 的教训：直接按 key 推导 jp 会改错。
  `西安` key=xian，可切成 1 个音节(xian) 或 2 个音节(xi|an)。
  我们取到 1 音节 → 推导出 "x"，但正确 jp 是 "xa"。这类必须跳过。

守卫规则（三条全满足才修）：
  1. key 能整串切成音节                      （是纯全拼）
  2. 切出的音节数 == 汉字字数                 （排除 xian 这类歧义切分）
  3. 现有 jp != 按音节首字母推导的 jp

用法:
  python3 jp_audit2.py <db路径>           只统计
  python3 jp_audit2.py <db路径> --fix     直接修（先自动备份 .bak_jpfix）
"""
import re
import shutil
import sqlite3
import sys
from collections import Counter

import os
SRC = os.environ.get("SYLL_SRC", "/root/prisir_ime_build/src/syllable.rs")
SYLLABLES = sorted(set(re.search(r'const SYLLABLE_LIST: &str = "(.*?)";',
                                 open(SRC, encoding="utf-8").read(), re.S).group(1).split()),
                   key=len, reverse=True)


def segments(s):
    n = len(s)
    can = [False] * (n + 1)
    can[n] = True
    for i in range(n - 1, -1, -1):
        for k in SYLLABLES:
            if s.startswith(k, i) and can[i + len(k)]:
                can[i] = True
                break
    if not can[0]:
        return None
    out, i = [], 0
    while i < n:
        for k in SYLLABLES:
            if s.startswith(k, i) and can[i + len(k)]:
                out.append(k)
                i += len(k)
                break
        else:
            return None
    return out


_cache = {}


def derive(key):
    """→ (expected_jp, 是否可用) 。不可用 = 切不开或音节数≠字数。"""
    if key in _cache:
        return _cache[key]
    segs = segments(key)
    _cache[key] = (None, False) if segs is None else (None, None)
    if segs is None:
        return None, False
    v = "".join(x[0] for x in segs)
    _cache[key] = (v, len(segs))
    return v, len(segs)


def main():
    db = sys.argv[1]
    do_fix = "--fix" in sys.argv

    if do_fix:
        shutil.copyfile(db, db + ".bak_jpfix")
        print("已备份 → %s.bak_jpfix" % db)

    con = sqlite3.connect(db)
    total = a_ok = a_bad = a_ambig = b_type = empty = 0
    ex = Counter()
    samples = []
    updates = []

    for rowid, jp, key, value in con.execute("SELECT rowid, jp, key, value FROM phrase"):
        total += 1
        exp, nseg = derive(key)
        if exp is None:
            b_type += 1
            continue
        # 守卫 2：音节数必须等于汉字字数，否则是歧义切分，跳过
        if nseg != len(value):
            a_ambig += 1
            continue
        cur = (jp or "").strip()
        if not cur:
            empty += 1
            updates.append((rowid, exp))
            continue
        if cur == exp:
            a_ok += 1
        else:
            a_bad += 1
            ex[(cur, exp)] += 1
            if len(samples) < 15:
                samples.append((key, value, cur, exp))
            updates.append((rowid, exp))

    print("总行数                 : %d" % total)
    print("可判定 A 类             : %d" % (a_ok + a_bad + empty))
    print("  jp 正确               : %d (%.3f%%)" % (a_ok, 100.0 * a_ok / max(a_ok + a_bad + empty, 1)))
    print("  jp 不一致            : %d" % a_bad)
    print("  jp 为空              : %d" % empty)
    print("跳过：key 非全拼       : %d" % b_type)
    print("跳过：切分歧义(音节数≠字数): %d" % a_ambig)
    print()
    print("不一致 TOP8 (现 → 应):")
    for (got, exp), n in ex.most_common(8):
        print("  %-10s → %-10s %d 行" % (got, exp, n))
    print()
    print("样例:")
    for key, value, got, exp in samples:
        print("  %-24s %-10s jp=%-10s 应=%-10s" % (key, value, got, exp))

    print("\n待修条数: %d" % len(updates))
    if do_fix and updates:
        con.executemany("UPDATE phrase SET jp=? WHERE rowid=?", [(e, r) for r, e in updates])
        con.commit()
        print("已写入 %d 条" % len(updates))
    con.close()


if __name__ == "__main__":
    main()
