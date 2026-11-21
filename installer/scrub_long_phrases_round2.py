#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""第二轮狠削：清掉领域导入带来的「看起来像词、其实是维基标题/工种全称/网文句」的长条目。

背景（已核实，不是引擎运行时机械组词）:
  - weight=6000  ≈ zhwiki 分类标题导入（物理/数学/计科… term_domain.tier=C）
  - weight=20000 ≈ THUOCL/funNLP 职业+医学词表（工种全称、药名长剂型）
  - weight=100/87 ≈ 网文句/歌词/产品名倾销
  引擎 query 路径不会把两短词拼成这些长条；怪词是入库源本身。

删除规则（命中即删；保护见 protect()）:
  A) weight=6000 且 L>=8          — 维基长标题
  B) weight=20000 且 L>=8         — 职业/医学超长专名
  C) weight=100 且 L>=9           — 句式/外国人名长串（L=8 暂留，人名大学等）
  D) weight=87 且 L>=8            — 软硬件产品倾销
  E) L>=11 且 weight<=3000        — 残留冷法规/机构全称
  F) L>=9 且 weight<=150          — 残余冷长词
  G) L>=8 且 weight<=30           — 极冷长词

用法:
  python installer/scrub_long_phrases_round2.py --db installer/stage/models/ciku.db --dry-run
  python installer/scrub_long_phrases_round2.py --db installer/stage/models/ciku.db --apply
"""
from __future__ import annotations

import argparse
import datetime as dt
import os
import shutil
import sqlite3
import sys
import time


def protect(value: str, weight: int) -> bool:
    if "Prisir" in value or "Babelspan" in value or "babelspan" in value.lower():
        return True
    if weight >= 50000:
        return True
    # 注意：weight=20000 是职业/医学词表倾销，不是「超高频」，不能保护
    # weight=9999 多为国名全称；其它 >=10000 的短词当口号保护
    if weight == 9999 and len(value) <= 13:
        return True
    if weight >= 10000 and weight != 20000 and len(value) <= 10:
        return True
    return False


def reason(value: str, weight: int) -> str | None:
    w = int(weight or 0)
    L = len(value)
    if protect(value, w) or L < 8:
        return None
    # L=7 的 weight=6000（如「相对论量子力学」）先留；从 L>=8 起削维基长标题
    if w == 6000:
        return "A_wiki6000_L>=8"
    if w == 20000:
        return "B_occ_med20000_L>=8"
    if w == 100 and L >= 9:
        return "C_w100_L>=9"
    if w == 87:
        return "D_w87_product_L>=8"
    if L >= 11 and w <= 3000:
        return "E_L>=11_w<=3000"
    if L >= 9 and w <= 150:
        return "F_L>=9_w<=150"
    if L >= 8 and w <= 30:
        return "G_L>=8_w<=30"
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
        "SELECT value, key, weight FROM phrase WHERE length(value) >= 8"
    ):
        r = reason(value, weight)
        if r:
            victims.append((value, key, int(weight or 0), r))

    from collections import Counter

    print(f"[scan] phrase={before:,} candidates={len(victims):,} ({time.time()-t0:.1f}s)")
    for k, n in Counter(r for *_, r in victims).most_common():
        print(f"  {k}: {n}")
    print("[sample]")
    for v, k, w, r in sorted(victims, key=lambda x: (-len(x[0]), x[2]))[:15]:
        print(f"  [{len(v)}] w={w} ({r}) {v}")

    report = os.path.join(
        os.path.dirname(a.db),
        f"scrub_round2_{dt.datetime.now().strftime('%Y%m%d_%H%M%S')}.txt",
    )
    with open(report, "w", encoding="utf-8") as f:
        f.write(f"# round2 {dt.datetime.now().isoformat()} n={len(victims)}\n")
        for v, k, w, r in victims:
            f.write(f"{r}\t{w}\t{len(v)}\t{k}\t{v}\n")
    print(f"[report] {report}")

    if a.dry_run:
        print("[dry-run] no changes")
        conn.close()
        return 0

    bak = a.db + f".bak_before_round2_{dt.datetime.now().strftime('%Y%m%d_%H%M%S')}"
    print(f"[backup] {bak}")
    shutil.copy2(a.db, bak)

    conn.execute("BEGIN")
    deleted = 0
    for v, k, w, r in victims:
        deleted += conn.execute(
            "DELETE FROM phrase WHERE value=? AND key=?", (v, k)
        ).rowcount
    # 同步清 term_domain 里已删的长冷词（避免元数据悬挂）
    conn.execute(
        """
        DELETE FROM term_domain
        WHERE length(value) >= 8
          AND value NOT IN (SELECT value FROM phrase)
        """
    )
    conn.commit()
    after = conn.execute("SELECT COUNT(*) FROM phrase").fetchone()[0]
    td = conn.execute("SELECT COUNT(*) FROM term_domain").fetchone()[0]
    print(f"[delete] phrase {before:,} → {after:,} (removed {deleted:,}); term_domain now {td:,}")

    print("[vacuum]…")
    t1 = time.time()
    conn.execute("VACUUM")
    conn.execute("ANALYZE")
    conn.close()
    print(f"[done] {time.time()-t1:.1f}s size={os.path.getsize(a.db)/1e6:.1f}MB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
