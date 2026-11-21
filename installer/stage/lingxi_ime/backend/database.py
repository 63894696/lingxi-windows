# -*- coding: utf-8 -*-
"""词库管理 — 统一后端
从 Yzime SQLite 词库迁移，支持拼音/五笔/自定义词库
"""
import sqlite3
import os
import glob
from typing import List, Tuple, Optional


class CikuDatabase:
    """词库数据库（统一后端）"""

    def __init__(self, db_path: Optional[str] = None):
        if db_path is None:
            # 默认从 Yzime 迁移
            db_path = self._find_yzime_db()
        self.db_path = db_path
        self._conn = None

    def _find_yzime_db(self) -> str:
        """查找词库路径（优先灵犀·拼音自己的 ciku.db）"""
        # 优先用灵犀·拼音自己的词库
        own_db = os.path.join(os.path.dirname(os.path.abspath(__file__)), "ciku.db")
        if os.path.exists(own_db):
            return own_db
        # fallback: Yzime 词库
        downloads = r"C:\Users\Administrator\Downloads"
        yzime_dirs = [d for d in glob.glob(os.path.join(downloads, "Yzime*")) if os.path.isdir(d)]
        if yzime_dirs:
            return os.path.join(yzime_dirs[0], "Data", "ciku.db")
        return ""

    def connect(self):
        """连接数据库（check_same_thread=False：输入法的查询/学习可能在不同线程触发）"""
        if self._conn is None:
            self._conn = sqlite3.connect(self.db_path, check_same_thread=False)
        return self._conn

    def close(self):
        """关闭连接"""
        if self._conn:
            self._conn.close()
            self._conn = None

    def all_phrase_rows(self) -> List[Tuple[str, str, int]]:
        """全部词组 (key, value, weight)，供内存索引构建。"""
        conn = self.connect()
        return conn.cursor().execute("SELECT key, value, weight FROM phrase").fetchall()

    def all_single_char_rows(self) -> List[Tuple[str, str, int]]:
        """全部单字 (key, value, weight)，供内存索引构建。"""
        conn = self.connect()
        return conn.cursor().execute(
            "SELECT key, value, weight FROM pinyin WHERE LENGTH(value)=1").fetchall()

    def query_pinyin(self, pinyin: str, limit: int = 10) -> List[Tuple[str, int]]:
        """查询拼音单字（pinyin 表上游混入约 31 万词组，必须 LENGTH(value)=1 过滤，
        否则“诉案/谜案”这类词组会盖掉“算/面”等单字）"""
        conn = self.connect()
        cursor = conn.cursor()
        cursor.execute(
            "SELECT value, weight FROM pinyin WHERE key=? AND LENGTH(value)=1 "
            "ORDER BY weight DESC LIMIT ?",
            (pinyin, limit)
        )
        return self._filter_blacklist(cursor.fetchall())

    def query_pinyin_jp(self, jp: str, limit: int = 10) -> List[Tuple[str, int]]:
        """查询简拼单字（同样过滤多字词组）"""
        conn = self.connect()
        cursor = conn.cursor()
        cursor.execute(
            "SELECT value, weight FROM pinyin WHERE jp=? AND LENGTH(value)=1 "
            "ORDER BY weight DESC LIMIT ?",
            (jp, limit)
        )
        return self._filter_blacklist(cursor.fetchall())

    def query_wubi(self, wubi: str, limit: int = 10) -> List[Tuple[str, int]]:
        """查询五笔候选词（预留）"""
        conn = self.connect()
        cursor = conn.cursor()
        cursor.execute(
            "SELECT value, weight FROM wubi86 WHERE key=? ORDER BY weight DESC LIMIT ?",
            (wubi, limit)
        )
        return self._filter_blacklist(cursor.fetchall())

    def get_all_pinyin_keys(self) -> List[str]:
        """获取所有拼音 key（用于切分）"""
        conn = self.connect()
        cursor = conn.cursor()
        cursor.execute("SELECT DISTINCT key FROM pinyin")
        return [r[0] for r in cursor.fetchall()]

    # ---------- 词组/短语支持 ----------
    def init_phrase_table(self):
        """初始化词组表（允许同音多词，不用 PRIMARY KEY）"""
        conn = self.connect()
        cursor = conn.cursor()
        # 检查表是否存在
        cursor.execute("SELECT name FROM sqlite_master WHERE type='table' AND name='phrase'")
        if not cursor.fetchone():
            cursor.execute("""
                CREATE TABLE phrase (
                    jp TEXT,
                    key TEXT,
                    value TEXT,
                    weight INTEGER DEFAULT 1
                )
            """)
            cursor.execute("CREATE INDEX idx_phrase_key ON phrase(key)")
            cursor.execute("CREATE INDEX idx_phrase_weight ON phrase(weight DESC)")
            conn.commit()

    def init_blacklist_whitelist_tables(self):
        """初始化黑白名单表。"""
        conn = self.connect()
        cursor = conn.cursor()
        cursor.execute("""
            CREATE TABLE IF NOT EXISTS word_blacklist (
                value TEXT PRIMARY KEY,
                reason TEXT,
                added_at TEXT DEFAULT (datetime('now'))
            )
        """)
        cursor.execute("""
            CREATE TABLE IF NOT EXISTS word_whitelist (
                value TEXT PRIMARY KEY,
                reason TEXT,
                added_at TEXT DEFAULT (datetime('now'))
            )
        """)
        conn.commit()

    # ---------- 黑白名单 CRUD ----------
    def add_to_blacklist(self, value: str, reason: str = ""):
        """将词组加入黑名单，查询时将被过滤。"""
        conn = self.connect()
        cursor = conn.cursor()
        cursor.execute(
            "INSERT OR REPLACE INTO word_blacklist (value, reason) VALUES (?, ?)",
            (value, reason))
        conn.commit()

    def remove_from_blacklist(self, value: str):
        """从黑名单移除。"""
        conn = self.connect()
        cursor = conn.cursor()
        cursor.execute("DELETE FROM word_blacklist WHERE value=?", (value,))
        conn.commit()

    def is_blacklisted(self, value: str) -> bool:
        """判断词组是否在黑名单中。"""
        conn = self.connect()
        cursor = conn.cursor()
        row = cursor.execute(
            "SELECT 1 FROM word_blacklist WHERE value=?", (value,)).fetchone()
        return row is not None

    def add_to_whitelist(self, value: str, reason: str = ""):
        """将词组加入白名单，查询时优先保留。"""
        conn = self.connect()
        cursor = conn.cursor()
        cursor.execute(
            "INSERT OR REPLACE INTO word_whitelist (value, reason) VALUES (?, ?)",
            (value, reason))
        conn.commit()

    def remove_from_whitelist(self, value: str):
        """从白名单移除。"""
        conn = self.connect()
        cursor = conn.cursor()
        cursor.execute("DELETE FROM word_whitelist WHERE value=?", (value,))
        conn.commit()

    def is_whitelisted(self, value: str) -> bool:
        """判断词组是否在白名单中。"""
        conn = self.connect()
        cursor = conn.cursor()
        row = cursor.execute(
            "SELECT 1 FROM word_whitelist WHERE value=?", (value,)).fetchone()
        return row is not None

    def get_blacklist(self) -> List[Tuple[str, str]]:
        """获取全部黑名单 [(value, reason)]。"""
        conn = self.connect()
        cursor = conn.cursor()
        return cursor.execute("SELECT value, reason FROM word_blacklist").fetchall()

    def get_whitelist(self) -> List[Tuple[str, str]]:
        """获取全部白名单 [(value, reason)]。"""
        conn = self.connect()
        cursor = conn.cursor()
        return cursor.execute("SELECT value, reason FROM word_whitelist").fetchall()

    def _filter_blacklist(self, rows: List[Tuple[str, int]]) -> List[Tuple[str, int]]:
        """过滤掉黑名单中的词组。"""
        if not rows:
            return rows
        conn = self.connect()
        cursor = conn.cursor()
        # 批量检查黑名单，避免逐条查询
        values = [r[0] for r in rows]
        placeholders = ",".join("?" for _ in values)
        blacked = set(
            r[0] for r in cursor.execute(
                f"SELECT value FROM word_blacklist WHERE value IN ({placeholders})",
                values).fetchall()
        )
        return [(v, w) for v, w in rows if v not in blacked]

    def query_phrase(self, pinyin: str, limit: int = 10) -> List[Tuple[str, int]]: 
        """查询词组（全拼 key 精确）"""
        conn = self.connect()
        cursor = conn.cursor()
        cursor.execute(
            "SELECT value, weight FROM phrase WHERE key=? ORDER BY weight DESC LIMIT ?",
            (pinyin, limit)
        )
        return self._filter_blacklist(cursor.fetchall())

    def query_phrase_by_jp(self, jp: str, limit: int = 20) -> List[Tuple[str, int]]:
        """纯简拼词组：jp 精确（sj→世界/时间）"""
        conn = self.connect()
        cursor = conn.cursor()
        cursor.execute(
            "SELECT value, weight FROM phrase WHERE jp=? ORDER BY weight DESC LIMIT ?",
            (jp, limit)
        )
        return self._filter_blacklist(cursor.fetchall())

    @staticmethod
    def _next_prefix(prefix: str) -> str:
        """前缀的后继串：shij -> shik，用于 key >= p AND key < next(p) 走索引的范围查询。
        （SQLite LIKE 'x%' 大小写不敏感、不走 BINARY 索引，范围比较才走索引）"""
        if not prefix:
            return prefix
        chars = list(prefix)
        i = len(chars) - 1
        while i >= 0 and chars[i] == "z":
            i -= 1
        if i < 0:
            return prefix + "a"  # 全 z 的边界，近似即可
        chars[i] = chr(ord(chars[i]) + 1)
        return "".join(chars[: i + 1])

    def query_phrase_by_key_prefix(self, prefix: str, limit: int = 15) -> List[Tuple[str, int]]:
        """前全拼+后首字母：key 前缀（shij→世界/实际）。范围比较走 idx_phrase_key。"""
        conn = self.connect()
        cursor = conn.cursor()
        cursor.execute(
            "SELECT value, weight FROM phrase WHERE key >= ? AND key < ? "
            "ORDER BY weight DESC LIMIT ?",
            (prefix, self._next_prefix(prefix), limit)
        )
        return self._filter_blacklist(cursor.fetchall())

    def query_phrase_reverse_mixed(self, first: str, rest: str, limit: int = 15) -> List[Tuple[str, int]]:
        """反向混拼：jp 首字母=first（范围走 idx_phrase_jp），结果集内 Python 过滤 key 后缀。
        key 后缀 LIKE '%rest' 无法走索引，故先按 jp 首字母缩小范围再内存过滤。"""
        conn = self.connect()
        cursor = conn.cursor()
        rows = cursor.execute(
            "SELECT value, key, weight FROM phrase WHERE jp >= ? AND jp < ? "
            "ORDER BY weight DESC LIMIT ?",
            (first, self._next_prefix(first), limit * 20)
        ).fetchall()
        out = [(val, w) for val, key, w in rows if key.endswith(rest)]
        return self._filter_blacklist(out[:limit])

    def add_phrase(self, pinyin: str, phrase: str, weight: int = 1):
        """添加词组"""
        conn = self.connect()
        cursor = conn.cursor()
        jp = self._to_jianpin(pinyin)
        cursor.execute(
            "INSERT OR REPLACE INTO phrase (jp, key, value, weight) VALUES (?, ?, ?, ?)",
            (jp, pinyin, phrase, weight)
        )
        conn.commit()

    def add_user_word(self, pinyin: str, word: str, weight: int = 1):
        """学习用户选择：多字词组写 phrase 表，单字才更新 pinyin 权重。
        旧逻辑一律写 pinyin，把词组混进单字表造成污染（如 suan→诉案）。"""
        if len(word) > 1:
            # 词组：写 phrase 表（存在则加权）
            conn = self.connect()
            cursor = conn.cursor()
            cursor.execute(
                "SELECT weight FROM phrase WHERE key=? AND value=?",
                (pinyin, word)
            )
            row = cursor.fetchone()
            if row:
                cursor.execute(
                    "UPDATE phrase SET weight=? WHERE key=? AND value=?",
                    (row[0] + weight, pinyin, word)
                )
                conn.commit()
            else:
                self.add_phrase(pinyin, word, weight=weight)
            return
        # 单字：更新 pinyin 权重
        conn = self.connect()
        cursor = conn.cursor()
        cursor.execute(
            "SELECT weight FROM pinyin WHERE key=? AND value=?",
            (pinyin, word)
        )
        row = cursor.fetchone()
        if row:
            cursor.execute(
                "UPDATE pinyin SET weight=? WHERE key=? AND value=?",
                (row[0] + weight, pinyin, word)
            )
        else:
            jp = self._to_jianpin(pinyin)
            cursor.execute(
                "INSERT INTO pinyin (jp, key, value, weight) VALUES (?, ?, ?, ?)",
                (jp, pinyin, word, weight)
            )
        conn.commit()

    def _to_jianpin(self, pinyin: str) -> str:
        """转简拼（每个音节首字母）。用标准音节表切分后取各音节首字母。
        旧实现只取首字母是坏的(186万词 jp 全错已重建),新词必须用正确简拼。"""
        try:
            from .engine import CandidateEngine
            keys = CandidateEngine._SYLLABLES
            segs = []
            i = 0
            s = pinyin
            ordered = sorted(keys, key=len, reverse=True)
            while i < len(s):
                for k in ordered:
                    if s.startswith(k, i):
                        segs.append(k)
                        i += len(k)
                        break
                else:
                    i += 1
            return "".join(x[0] for x in segs) if segs else (pinyin[0] if pinyin else "")
        except Exception:
            return pinyin[0] if pinyin else ""

    # ---------- 术语库编辑器 CRUD ----------
    def dict_list(self, keyword: str = "", limit: int = 100, offset: int = 0) -> List[Tuple[str, str, int]]:
        """列出词组（可按拼音或词搜索），返回 [(key, value, weight)]。"""
        conn = self.connect()
        cursor = conn.cursor()
        if keyword:
            like = f"%{keyword}%"
            cursor.execute(
                "SELECT key, value, weight FROM phrase WHERE key LIKE ? OR value LIKE ? "
                "ORDER BY weight DESC LIMIT ? OFFSET ?",
                (like, like, limit, offset))
        else:
            cursor.execute(
                "SELECT key, value, weight FROM phrase ORDER BY weight DESC LIMIT ? OFFSET ?",
                (limit, offset))
        return cursor.fetchall()

    def dict_max_weight(self, pinyin: str) -> int:
        """同 key 下的最高词频（用于保存时自动提频，保证新词排第一）。"""
        conn = self.connect()
        row = conn.cursor().execute(
            "SELECT MAX(weight) FROM phrase WHERE key=?", (pinyin,)).fetchone()
        return row[0] if row and row[0] is not None else 0

    def dict_count(self, keyword: str = "") -> int:
        """词组总数（用于分页）。"""
        conn = self.connect()
        cursor = conn.cursor()
        if keyword:
            like = f"%{keyword}%"
            return cursor.execute(
                "SELECT COUNT(*) FROM phrase WHERE key LIKE ? OR value LIKE ?",
                (like, like)).fetchone()[0]
        return cursor.execute("SELECT COUNT(*) FROM phrase").fetchone()[0]

    def dict_upsert(self, pinyin: str, word: str, weight: int):
        """新增或更新词组（key+value 唯一语义：存在则改权重）。"""
        conn = self.connect()
        cursor = conn.cursor()
        jp = self._to_jianpin(pinyin)
        row = cursor.execute(
            "SELECT weight FROM phrase WHERE key=? AND value=?", (pinyin, word)).fetchone()
        if row:
            cursor.execute(
                "UPDATE phrase SET weight=?, jp=? WHERE key=? AND value=?",
                (weight, jp, pinyin, word))
        else:
            cursor.execute(
                "INSERT INTO phrase (jp, key, value, weight) VALUES (?,?,?,?)",
                (jp, pinyin, word, weight))
        conn.commit()

    def dict_delete(self, pinyin: str, word: str):
        """删除词组。"""
        conn = self.connect()
        cursor = conn.cursor()
        cursor.execute("DELETE FROM phrase WHERE key=? AND value=?", (pinyin, word))
        conn.commit()

    # ---------- 多拼音别名（同一词组可对应多个拼音 key）----------
    def dict_add_alias(self, pinyin: str, word: str, weight: int = 1):
        """为已有词组添加一个拼音别名。

        同一个 word 可以对应多个 pinyin key，查询任意一个 key 都能打出该词组。
        例如：dict_add_alias("beijing", "北京") 后，打 "beijing" 都能出 "北京"。
        """
        conn = self.connect()
        cursor = conn.cursor()
        jp = self._to_jianpin(pinyin)
        # 如果该 key+value 已存在，只更新权重取最大
        row = cursor.execute(
            "SELECT weight FROM phrase WHERE key=? AND value=?", (pinyin, word)).fetchone()
        if row:
            new_weight = max(row[0], weight)
            cursor.execute(
                "UPDATE phrase SET weight=?, jp=? WHERE key=? AND value=?",
                (new_weight, jp, pinyin, word))
        else:
            cursor.execute(
                "INSERT INTO phrase (jp, key, value, weight) VALUES (?,?,?,?)",
                (jp, pinyin, word, weight))
        conn.commit()

    def dict_list_aliases(self, word: str) -> List[Tuple[str, int]]:
        """列出某个词组的所有拼音别名 [(pinyin, weight)]。"""
        conn = self.connect()
        cursor = conn.cursor()
        return cursor.execute(
            "SELECT key, weight FROM phrase WHERE value=? ORDER BY weight DESC",
            (word,)).fetchall()

    def dict_remove_alias(self, pinyin: str, word: str):
        """删除某个拼音别名（不删除词组本身）。"""
        conn = self.connect()
        cursor = conn.cursor()
        cursor.execute("DELETE FROM phrase WHERE key=? AND value=?", (pinyin, word))
        conn.commit()

    # ---------- 自动别名学习（基于每个字首字母）----------
    def _get_char_primary_letter(self, char: str) -> Optional[str]:
        """获取单个汉字的拼音首字母。

        从 pinyin 表查询该字的标准拼音，取权重最高的一条，返回其首字母。
        例如："世" → "shi" → "s"，"界" → "jie" → "j"。
        """
        conn = self.connect()
        cursor = conn.cursor()
        row = cursor.execute(
            "SELECT key FROM pinyin WHERE value=? AND LENGTH(value)=1 "
            "ORDER BY weight DESC LIMIT 1",
            (char,)
        ).fetchone()
        if row:
            return row[0][0].lower()
        return None

    def auto_add_alias_for_word(self, word: str, base_pinyin: str = "", weight: int = 1) -> List[str]:
        """为词组自动添加基于每个字首字母的拼音别名。

        用户习惯：对词组中的前一个字或后一个字，用其拼音首字母代替，
        例如："世界" → 别名 "s"（世的首字母）、"j"（界的首字母）。
        用户打 "s" 或 "j" 时，"世界" 都会作为候选词出现（与其他同首字母词组一起翻页）。

        返回实际新增的别名列表。
        """
        added: List[str] = []
        existing = {k for k, _ in self.dict_list_aliases(word)}
        for char in word:
            letter = self._get_char_primary_letter(char)
            if letter and letter not in existing:
                self.dict_add_alias(letter, word, weight=weight)
                added.append(letter)
                existing.add(letter)
        return added

    def get_alias_suggestions(self, word: str) -> List[str]:
        """建议为词组添加的单字母别名（未已存在的）。"""
        existing = {k for k, _ in self.dict_list_aliases(word)}
        suggestions: List[str] = []
        for char in word:
            letter = self._get_char_primary_letter(char)
            if letter and letter not in existing:
                suggestions.append(letter)
        return suggestions


# 全局单例
_db = None

def get_db() -> CikuDatabase:
    global _db
    if _db is None:
        _db = CikuDatabase()
        _db.init_phrase_table()  # 初始化词组表
        _db.init_blacklist_whitelist_tables()  # 初始化黑白名单表
    return _db
