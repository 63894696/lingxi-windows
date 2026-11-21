#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""筛删非日常长词组（机械组合 / 冷僻法规书名 / 古文碎句），为查询提速减负。

规则（任一命中且未受保护则删）:
  1) L >= 14                         — 超长，几乎无人整词输入
  2) L >= 12 且 weight <= 500         — 长且不够热
  3) L >= 8  且 weight <= 50
     且匹配：法规/办法/条例/书名后缀 / 公司全称 / 中华人民共和国…加长书名 / 文言虚词堆砌
  4) L >= 9  且 weight <= 30 且公司/法规/书名后缀

保护（永不删）:
  - 含 Prisir
  - weight >= 10000
  - weight >= 2000 且 L <= 10（高频口号/机构短称）

用法:
  python installer/scrub_long_phrases.py --db installer/stage/models/ciku.db --dry-run
  python installer/scrub_long_phrases.py --db installer/stage/models/ciku.db --apply
"""
from __future__ import annotations

import argparse
import datetime as dt
import os
import shutil
import sqlite3
import sys
import time

BUREAU_END = (
    "管理办法", "暂行办法", "实施细则", "实施办法", "试行办法", "暂行规定",
    "管理规定", "管理条例", "保护条例", "监督条例", "组织法", "选举法",
    "导则", "公约", "法典", "考试必读", "注释本", "执照管理规则", "就业办法",
    "工作领导小组", "工作领导小组办公室",
)
BOOK_END = (
    "汇编", "指南", "概论", "概要", "教程", "手册", "读本", "选编",
    "论文集", "研究纲要", "文件解读", "学习辅导", "应用指南", "实务指南",
    "基本问题研究", "法规目录",
)
CO_END = ("有限公司", "股份有限公司", "有限责任公司")
CLASSICAL_CHARS = set("之乎者也矣焉哉兮耳矣")


def protect(value: str, weight: int) -> bool:
    if "Prisir" in value or "prisir" in value:
        return True
    if weight >= 10000:
        return True
    if weight >= 2000 and len(value) <= 10:
        return True
    return False


def ends_any(value: str, suffixes: tuple[str, ...]) -> bool:
    return any(value.endswith(s) for s in suffixes)


def is_classical(value: str) -> bool:
    return sum(1 for ch in value if ch in CLASSICAL_CHARS) >= 2


def should_delete(value: str, weight: int) -> str | None:
    """Return reason code or None."""
    w = weight or 0
    L = len(value)
    if L < 8 or protect(value, w):
        return None
    if L >= 14:
        return "L>=14"
    if L >= 12 and w <= 500:
        return "L12_w<=500"
    mech = (
        ends_any(value, BUREAU_END)
        or ends_any(value, BOOK_END)
        or ends_any(value, CO_END)
        or (value.startswith("中华人民共和国") and L > 9)
        or is_classical(value)
    )
    if L >= 8 and w <= 50 and mech:
        return "L8_w<=50_mech"
    if L >= 9 and w <= 30 and (
        ends_any(value, BUREAU_END) or ends_any(value, BOOK_END) or ends_any(value, CO_END)
    ):
        return "L9_w<=30_mech"
    return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", required=True)
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--report", default="")
    a = ap.parse_args()
    if a.dry_run == a.apply:
        print("指定 --dry-run 或 --apply 之一", file=sys.stderr)
        return 2
    if not os.path.isfile(a.db):
        print(f"[x] 词库不存在: {a.db}", file=sys.stderr)
        return 1

    conn = sqlite3.connect(a.db)
    conn.row_factory = sqlite3.Row
    before_n = conn.execute("SELECT COUNT(*) FROM phrase").fetchone()[0]
    t0 = time.time()
    victims: list[tuple[str, str, int, str]] = []
    for row in conn.execute(
        "SELECT value, key, weight FROM phrase WHERE length(value) >= 8"
    ):
        reason = should_delete(row["value"], int(row["weight"] or 0))
        if reason:
            victims.append((row["value"], row["key"], int(row["weight"] or 0), reason))

    from collections import Counter

    reasons = Counter(r for *_, r in victims)
    print(f"[scan] phrase={before_n:,}  candidates={len(victims):,}  ({time.time()-t0:.1f}s)")
    for k, n in sorted(reasons.items(), key=lambda x: -x[1]):
        print(f"  {k}: {n}")
    print("[sample]")
    for v, k, w, r in sorted(victims, key=lambda x: (-len(x[0]), x[2]))[:12]:
        print(f"  [{len(v)}] w={w} ({r}) {v}")

    report = a.report or os.path.join(
        os.path.dirname(a.db),
        f"scrub_long_phrases_{dt.datetime.now().strftime('%Y%m%d_%H%M%S')}.txt",
    )
    with open(report, "w", encoding="utf-8") as f:
        f.write(f"# scrub_long_phrases {dt.datetime.now().isoformat()}\n")
        f.write(f"# db={a.db}\n# candidates={len(victims)}\n")
        for v, k, w, r in victims:
            f.write(f"{r}\t{w}\t{len(v)}\t{k}\t{v}\n")
    print(f"[report] {report}")

    if a.dry_run:
        print("[dry-run] 未改库")
        conn.close()
        return 0

    # backup
    bak = a.db + f".bak_before_longscrub_{dt.datetime.now().strftime('%Y%m%d_%H%M%S')}"
    print(f"[backup] {bak}")
    shutil.copy2(a.db, bak)

    # delete by value+key (phrase 无 rowid 主键假设；用 pair 更稳)
    conn.execute("BEGIN")
    deleted = 0
    for v, k, w, r in victims:
        cur = conn.execute("DELETE FROM phrase WHERE value=? AND key=?", (v, k))
        deleted += cur.rowcount
    conn.commit()
    after_n = conn.execute("SELECT COUNT(*) FROM phrase").fetchone()[0]
    print(f"[delete] removed={deleted:,}  phrase {before_n:,} → {after_n:,}")

    print("[vacuum] …")
    t1 = time.time()
    conn.execute("VACUUM")
    conn.execute("ANALYZE")
    conn.close()
    print(f"[vacuum] done ({time.time()-t1:.1f}s) size={os.path.getsize(a.db)/1e6:.1f}MB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
