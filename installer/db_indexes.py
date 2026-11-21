#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""词库索引迁移（幂等）—— 打包前跑一次，给引擎的慢查询铺索引。

为什么需要 substr(key,-3) 这个表达式索引：
  反向混拼（首字母 + 后全拼，如 sjie → 世界）只能靠"jp 首字母范围 + key 后缀"查，
  原 SQL 扫 z 开头 17 万行再排序 → 'zo' 341ms。加 length(jp)=音节数 + 末 3 字表达式索引后
  降到 30~45ms（见 db.rs query_phrase_reverse_mixed 注释）。

代价：约 +26MB（词库 248MB → 274MB，安装包 +~10MB）。
可选：--no-tail3 只加轻量索引（体积敏感时可跳过，功能不受影响，只是那条路径慢）。

用法:
  python installer/db_indexes.py                    # 默认对 stage 词库执行
  python installer/db_indexes.py --db path/ciku.db
  python installer/db_indexes.py --check            # 只检查，不建
"""
import argparse
import os
import sqlite3
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_DB = os.path.join(HERE, "stage", "models", "ciku.db")

INDEXES = {
    # 反向混拼的末 3 字过滤（表达式索引，SQLite 3.9+）
    "idx_phrase_key_tail3": "CREATE INDEX IF NOT EXISTS idx_phrase_key_tail3 ON phrase(substr(key,-3), weight DESC)",
    # 词库窗按"词"前缀搜索（原来 LIKE '%词%' 全表扫 3.1 秒/次）
    "idx_phrase_value": "CREATE INDEX IF NOT EXISTS idx_phrase_value ON phrase(value)",
}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", default=DEFAULT_DB)
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--no-tail3", action="store_true")
    a = ap.parse_args()

    if not os.path.exists(a.db):
        print(f"[x] 词库不存在: {a.db}", file=sys.stderr)
        return 1

    conn = sqlite3.connect(a.db)
    existing = {r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='index'")}
    want = {} if a.no_tail3 else INDEXES

    missing = [n for n in want if n not in existing]
    if a.check:
        print(f"[check] {os.path.basename(a.db)} 已有索引 {len(existing)} 个；缺 {missing or '无'}")
        conn.close()
        return 0

    before = os.path.getsize(a.db)
    for name in missing:
        t0 = time.time()
        conn.execute(want[name])
        conn.commit()
        print(f"[+] {name} 建好（{time.time()-t0:.1f}s）")
    conn.close()
    after = os.path.getsize(a.db)
    print(f"[✓] 完成：{before/1e6:.0f}MB → {after/1e6:.0f}MB（+{(after-before)/1e6:.0f}MB）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
