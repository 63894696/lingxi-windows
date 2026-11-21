"""词库体检：全库扫描 phrase.jp 与 key 的一致性。

背景：简拼（每字首字母）依赖 jp 列。若 jp 录错，整串简拼查不到，
只能退化成分词解码，用户感知就是"打 zhrmghg 出不来 中华人民共和国"。

分类：
  A 类（可修）  key 是纯全拼（能整串切成音节）→ 期望 jp = 每个音节首字母
  B 类（跳过）  key 本身不是全拼（混拼/别名行，库里是 reverse-mixed 约定，
                如 key='sj', jp='s'），不能按 A 类规则改

用法:
  python3 jp_audit.py            # 只统计+样例
  python3 jp_audit.py --fix      # 生成修复 SQL 到 /root/jp_fix.sql（不直接改库）
"""
import re
import sqlite3
import sys
from collections import Counter

DB = "/usr/lib/fcitx5/prisir/ciku.db"
SRC = "/root/prisir_ime_build/src/syllable.rs"

text = open(SRC, encoding="utf-8").read()
SYLLABLES = sorted(set(re.search(r'const SYLLABLE_LIST: &str = "(.*?)";', text, re.S).group(1).split()),
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


cache = {}


def expected_jp(key):
    if key in cache:
        return cache[key]
    segs = segments(key)
    v = None if segs is None else "".join(x[0] for x in segs)
    cache[key] = v
    return v


con = sqlite3.connect(DB)
fix = "--fix" in sys.argv

total = 0
type_a = 0
type_b = 0
ok = 0
mismatch = Counter()
samples = []
empty_jp = 0

cur = con.execute("SELECT rowid, jp, key, value, weight FROM phrase")
for rowid, jp, key, value, weight in cur:
    total += 1
    exp = expected_jp(key)
    if exp is None:
        type_b += 1
        continue
    type_a += 1
    cur_jp = (jp or "").strip()
    if cur_jp == "":
        empty_jp += 1
        if len(samples) < 25:
            samples.append(("空jp", key, value, cur_jp, exp))
        continue
    if cur_jp == exp:
        ok += 1
    else:
        mismatch[(cur_jp, exp)] += 1
        if len(samples) < 25:
            samples.append(("不一致", key, value, cur_jp, exp))

print("总行数           : %d" % total)
print("A 类(纯全拼 key) : %d" % type_a)
print("B 类(key 非全拼) : %d  （混拼/别名行，不参与比对）" % type_b)
print("A 类 jp 正确     : %d  (%.3f%%)" % (ok, 100.0 * ok / max(type_a, 1)))
print("A 类 jp 为空     : %d" % empty_jp)
print("A 类 jp 不一致   : %d" % sum(mismatch.values()))
print()
print("不一致 TOP10 (现 jp → 应 jp):")
for (got, exp), n in mismatch.most_common(10):
    print("  %-12s → %-12s  %d 行" % (got or "<空>", exp, n))
print()
print("样例:")
for kind, key, value, got, exp in samples[:25]:
    print("  [%s] key=%-26s value=%-12s jp=%r 应=%r" % (kind, key, value, got, exp))

if fix:
    with open("/root/jp_fix.sql", "w", encoding="utf-8") as f:
        f.write("BEGIN;\n")
        cur2 = con.execute("SELECT rowid, jp, key FROM phrase")
        n = 0
        for rowid, jp, key in cur2:
            exp = expected_jp(key)
            if exp is None:
                continue
            if (jp or "").strip() != exp:
                f.write("UPDATE phrase SET jp=%s WHERE rowid=%d;\n"
                        % ("'" + exp + "'", rowid))
                n += 1
        f.write("COMMIT;\n")
    print("\n已生成 /root/jp_fix.sql，共 %d 条 UPDATE" % n)
