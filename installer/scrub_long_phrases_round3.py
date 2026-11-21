#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""第三轮：清掉仍会在短前缀下冒出来的「怪长词」——古文诗句倾销、叠字废词、句式碎片、剂型药名。

背景:
  - 用户反馈 zhongg 出怪六字词；当前 stage/已装库均无「只會歐鮎改革」原文，
    但同族垃圾仍大量存在：weight=20000 的古诗残句、剂型药名；weight=100 的口语碎片。
  - round1/2 只砍 L>=8；L=5~7 的诗句/药名仍在。

删除规则（命中即删；protect 例外）:
  A) classical   — 文言虚词/诗句头（兮矣哉焉乎… / 安得/哀吾… / 「之」+文言共现）
  B) repeat_junk — 叠字废词（啊啊啊 / 平平仄仄…）
  C) sent_frag   — weight=100 且口语句式头尾（它的/把X起来/…一下）
  D) drug_form   — weight=20000 且以剂型结尾（胶囊/颗粒/注射液…）L>=5
  E) w100_L>=7   — weight=100 且 L>=7（网文句/外国人名长串续砍）

用法:
  python installer/scrub_long_phrases_round3.py --db installer/stage/models/ciku.db --dry-run
  python installer/scrub_long_phrases_round3.py --db installer/stage/models/ciku.db --apply
"""
from __future__ import annotations

import argparse
import datetime as dt
import os
import re
import shutil
import sqlite3
import sys
import time
from collections import Counter

CLASSICAL_CHARS = set("兮矣哉焉乎歟耳耶")
CLASSICAL_PREFIX = (
    "安得", "哀吾", "哀莫", "哀哀", "岂有", "何当", "愿君", "莫使",
    "欲穷", "不知", "但愿", "何以", "尔来", "君不见",
)
DRUG_SUFFIX = (
    "胶囊", "颗粒", "软膏", "乳膏", "眼膏", "注射液", "滴眼液",
    "分散片", "缓释片", "肠溶片", "泡腾片", "咀嚼片", "口服液",
    "滴剂", "栓剂", "贴膏",
)


def protect(value: str, weight: int) -> bool:
    if "Prisir" in value or "Babelspan" in value or "babelspan" in value.lower():
        return True
    if weight >= 50000:
        return True
    # 高频日常六字（有限责任公司等）不动
    if weight >= 10000 and weight not in (20000, 6000, 9999) and len(value) <= 8:
        return True
    if weight == 9999 and len(value) <= 13:
        return True
    return False


def is_classical(v: str) -> bool:
    if any(ch in v for ch in CLASSICAL_CHARS):
        return True
    if v.startswith(CLASSICAL_PREFIX):
        return True
    # 「之」+ 文言共现（哀民生之多艰 / 爱之必以其道）
    if "之" in v and any(x in v for x in ("而", "其", "无", "不", "以", "于", "於", "兮")):
        if re.search(r".之.", v):
            return True
    return False


def is_repeat_junk(v: str) -> bool:
    if len(v) >= 4 and len(set(v)) <= 2:
        return True
    if re.search(r"(.)\1{2,}", v):  # 同一字连续 ≥3
        return True
    if "平平" in v and "仄" in v:
        return True
    return False


def is_sentence_frag(v: str) -> bool:
    if v.startswith(("它的", "他的", "她的", "我的", "你的", "这是", "那是", "就是", "不是", "把")):
        return True
    if v.endswith(("的话", "一下", "起来", "下来", "上去", "过来", "下去", "出来")) and len(v) >= 5:
        return True
    return False


def is_drug_form(v: str) -> bool:
    return len(v) >= 5 and any(v.endswith(s) for s in DRUG_SUFFIX)


def reason(value: str, weight: int) -> str | None:
    w = int(weight or 0)
    L = len(value)
    if protect(value, w) or L < 5:
        return None
    # 只动导入倾销桶，避免误伤真实语料权重
    if w not in (100, 87, 20000, 6000) and not (w == 100):
        # classical/repeat 在 20000/100/6000 为主；也对 20000/100 开
        pass

    if w in (100, 87, 20000, 6000) and is_classical(value):
        return "A_classical"
    if w in (100, 87, 20000, 6000) and is_repeat_junk(value):
        return "B_repeat"
    if w == 100 and is_sentence_frag(value):
        return "C_frag"
    if w == 20000 and is_drug_form(value):
        return "D_drug"
    if w == 100 and L >= 7:
        return "E_w100_L>=7"
    if w == 87 and L >= 6:
        return "F_w87_L>=6"
    return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", required=True)
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--dry-run", action="store_true")
    g.add_argument("--apply", action="store_true")
    a = ap.parse_args()

    conn = sqlite3.connect(a.db)
    before = conn.execute("SELECT COUNT(*) FROM phrase").fetchone()[0]
    t0 = time.time()
    victims: list[tuple[str, str, int, str]] = []
    for value, key, weight in conn.execute(
        "SELECT value, key, weight FROM phrase WHERE length(value) >= 5 "
        "AND weight IN (100, 87, 20000, 6000)"
    ):
        r = reason(value, weight)
        if r:
            victims.append((value, key, int(weight or 0), r))

    print(f"[scan] phrase={before:,} candidates={len(victims):,} ({time.time()-t0:.1f}s)")
    for k, n in Counter(r for *_, r in victims).most_common():
        print(f"  {k}: {n}")
    print("[sample]")
    for v, k, w, r in sorted(victims, key=lambda x: (-len(x[0]), x[2]))[:20]:
        print(f"  [{len(v)}] w={w} ({r}) {v}")

    report = os.path.join(
        os.path.dirname(a.db),
        f"scrub_round3_{dt.datetime.now().strftime('%Y%m%d_%H%M%S')}.txt",
    )
    with open(report, "w", encoding="utf-8") as f:
        f.write(f"# round3 {dt.datetime.now().isoformat()} n={len(victims)}\n")
        for v, k, w, r in victims:
            f.write(f"{r}\t{w}\t{len(v)}\t{k}\t{v}\n")
    print(f"[report] {report}")

    if a.dry_run:
        print("[dry-run] no changes")
        conn.close()
        return 0

    bak = a.db + f".bak_before_round3_{dt.datetime.now().strftime('%Y%m%d_%H%M%S')}"
    print(f"[backup] {bak}")
    shutil.copy2(a.db, bak)

    conn.execute("BEGIN")
    deleted = 0
    for v, k, w, r in victims:
        deleted += conn.execute(
            "DELETE FROM phrase WHERE value=? AND key=?", (v, k)
        ).rowcount
    conn.execute(
        """
        DELETE FROM term_domain
        WHERE length(value) >= 5
          AND value NOT IN (SELECT value FROM phrase)
        """
    )
    conn.commit()
    after = conn.execute("SELECT COUNT(*) FROM phrase").fetchone()[0]
    print(f"[delete] phrase {before:,} → {after:,} (removed {deleted:,})")

    # sanity: zhongg 首候选应是中国
    top = conn.execute(
        "SELECT value, weight FROM phrase WHERE key >= 'zhongg' AND key < 'zhongh' "
        "ORDER BY weight DESC LIMIT 5"
    ).fetchall()
    print("[check zhongg]", top)

    print("[vacuum]…")
    t1 = time.time()
    conn.execute("VACUUM")
    conn.execute("ANALYZE")
    conn.close()
    print(f"[done] {time.time()-t1:.1f}s size={os.path.getsize(a.db)/1e6:.1f}MB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
