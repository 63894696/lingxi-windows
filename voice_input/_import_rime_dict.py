# -*- coding: utf-8 -*-
"""把 Rime 雾凇拼音词库导入灵犀·拼音 SQLite"""
import re
import sqlite3
import os

def parse_rime_dict(yaml_path):
    """解析 Rime YAML 词库，返回 [(pinyin, word, weight), ...]"""
    entries = []
    with open(yaml_path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            # 跳过注释和空行
            if not line or line.startswith("#"):
                continue
            # 跳过 YAML 头
            if line.startswith(("name:", "version:", "sort:", "...")):
                continue
            # 解析词条: 汉字\t拼音\t权重 (如: 阿乖\ta guai\t1)
            parts = line.split("\t")
            if len(parts) >= 2:
                word = parts[0]
                pinyin = parts[1].replace(" ", "").replace("'", "")  # 去掉空格和分词符
                weight = int(parts[2]) if len(parts) >= 3 else 1
                entries.append((pinyin, word, weight))
    return entries

def import_to_db(entries, db_path):
    """导入到 SQLite"""
    conn = sqlite3.connect(db_path)
    cursor = conn.cursor()
    # 创建 phrase 表（不用 PRIMARY KEY，允许同音多词）
    cursor.execute("DROP TABLE IF EXISTS phrase")
    cursor.execute("""
        CREATE TABLE phrase (
            jp TEXT,
            key TEXT,
            value TEXT,
            weight INTEGER DEFAULT 1
        )
    """)
    # 批量插入
    for pinyin, word, weight in entries:
        jp = pinyin[0] if pinyin else ""
        cursor.execute(
            "INSERT INTO phrase (jp, key, value, weight) VALUES (?, ?, ?, ?)",
            (jp, pinyin, word, weight)
        )
    conn.commit()
    # 创建索引加速查询
    cursor.execute("CREATE INDEX IF NOT EXISTS idx_phrase_key ON phrase(key)")
    cursor.execute("CREATE INDEX IF NOT EXISTS idx_phrase_weight ON phrase(weight DESC)")
    conn.commit()
    conn.close()
    print(f"imported {len(entries)} entries")

if __name__ == "__main__":
    base_dir = r"C:\Users\Administrator\voice_input\rime-ice\cn_dicts"
    db_path = r"C:\Users\Administrator\voice_input\lingxi_ime\backend\ciku.db"
    
    all_entries = []
    for fname in ["base.dict.yaml", "ext.dict.yaml", "tencent.dict.yaml"]:
        path = os.path.join(base_dir, fname)
        if os.path.exists(path):
            print(f"parsing {fname}...")
            entries = parse_rime_dict(path)
            all_entries.extend(entries)
            print(f"  {len(entries)} entries")
    
    print(f"total: {len(all_entries)}")
    import_to_db(all_entries, db_path)
