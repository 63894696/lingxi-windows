"""P1 解码器 Python 原型 —— 与 Rust lattice.rs 同算法，用于快速调参和诊断。
跑在目标机（有真实 ciku.db）。

用法:
  python3 p1_proto.py            # 跑用例，打印 top-1 和路径分解
  python3 p1_proto.py -v         # 额外打印每个跨度的命中
"""
import re
import sqlite3
import sys
from functools import lru_cache

DB = "file:/usr/lib/fcitx5/prisir/ciku.db?mode=ro"
SRC = "/root/prisir_ime_build/src/syllable.rs"

# ---------- 音节表（直接从 Rust 源码抽，保证一致） ----------
text = open(SRC, encoding="utf-8").read()
m = re.search(r'const SYLLABLE_LIST: &str = "(.*?)";', text, re.S)
SYLLABLES = sorted(set(m.group(1).split()), key=len, reverse=True)
INITIALS = set("bpmfdtnlgkhjqxrzcsyw")


def segments(s):
    """返回恰好消费整串的一个音节切分(list)或 None。"""
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
                out.append(k); i += len(k); break
        else:
            return None
    return out


def can_full(s):
    return segments(s) is not None


def max_word_chars(span):
    """跨度能承载的最大词字数。

    全拼跨度 = 音节数（shi 只能出单字，nihao 最多 2 字）；
    简拼/混拼跨度 = 字母数（每个字母至少 1 个汉字）。
    这条约束是整句解码不崩的关键：没有它，`shi` 会通过 key 前缀
    匹配出「实现」这类 2 字词，1 个音节吃掉 3 个字节的语义。
    """
    segs = segments(span)
    return len(segs) if segs else len(span)


def mixed_viable(s):
    n = len(s)
    can = [False] * (n + 1)
    can[n] = True
    for i in range(n - 1, -1, -1):
        for k in SYLLABLES:
            if s.startswith(k, i) and can[i + len(k)]:
                can[i] = True
                break
        if not can[i] and s[i] in INITIALS and can[i + 1]:
            can[i] = True
    return can[0]


con = sqlite3.connect(DB, uri=True)

MAX_SPAN = 24
BEAM = 4
LEN_BONUS = float(__import__("os").environ.get("LB","1.5"))
WORD_PENALTY = float(__import__("os").environ.get("WP","18.0"))
WP_LETTER = float(__import__("os").environ.get("WPL","12.0"))
# 每字单字母奖励：字母模式下，词字数 == 跨度字母数（一字母一汉字）时给的额外分。
BONUS_INIT = float(__import__("os").environ.get("BINIT","0"))
SPAN_LIMIT = int(__import__("os").environ.get("SLIM","6"))
JP_CHAR_LIMIT = 6

# 可切换的查询项
USE_KEY_PREFIX = True
USE_REVERSE_MIXED = True


@lru_cache(maxsize=100000)
def span_hits(span):
    max_chars = max_word_chars(span)
    # 短跨度（1-2 字母）放宽候选数：这些跨度对应「每字单字母」和简拼，
    # 候选词权重几乎持平（中华 500286 在 jp=zh 里只排第 9），
    # 卡在 6 会让正确路径根本不可达。长跨度保持 6，否则混拼会被长词淹没。
    lim = int(__import__("os").environ.get("SLIM_SHORT", "12")) if len(span) <= 2 else SPAN_LIMIT
    m = {}

    def put(w, wt):
        if w and len(w) <= max_chars:
            m[w] = max(m.get(w, 0), wt)

    for w, wt in con.execute(
        "SELECT value,weight FROM phrase WHERE key=? ORDER BY weight DESC LIMIT ?", (span, lim)
    ):
        put(w, wt)
    for w, wt in con.execute(
        "SELECT value,weight FROM phrase WHERE jp=? ORDER BY weight DESC LIMIT ?", (span, SPAN_LIMIT)
    ):
        put(w, wt)
    # key 前缀匹配只服务「混拼」（shij → shijie）。
    # 跨度太短时前缀会命中海量词（实测 `zh` → 制度/知道），把整句带偏，故要求 >=4。
    if USE_KEY_PREFIX and len(span) >= 4:
        hi = span[:-1] + chr(ord(span[-1]) + 1) if span[-1] < "z" else span + "{"
        for w, wt in con.execute(
            "SELECT value,weight FROM phrase WHERE key>=? AND key<? ORDER BY weight DESC LIMIT ?",
            (span, hi, SPAN_LIMIT),
        ):
            put(w, wt)
    if USE_REVERSE_MIXED and len(span) >= 2 and can_full(span[1:]):
        first = span[0]
        for w, wt in con.execute(
            "SELECT value,weight FROM phrase WHERE jp>=? AND jp<? ORDER BY weight DESC LIMIT ?",
            (first, chr(ord(first) + 1), 40),
        ):
            # 该 jp 对应的 key 是否以 first 开头、其余是全拼
            pass
    if len(span) == 1:
        for w, wt in con.execute(
            "SELECT value,weight FROM pinyin WHERE LENGTH(value)=1 AND jp=? ORDER BY weight DESC LIMIT ?",
            (span, JP_CHAR_LIMIT),
        ):
            put(w, wt)
    elif can_full(span):
        for w, wt in con.execute(
            "SELECT value,weight FROM pinyin WHERE LENGTH(value)=1 AND key=? ORDER BY weight DESC LIMIT ?",
            (span, JP_CHAR_LIMIT),
        ):
            put(w, wt)
    return sorted(m.items(), key=lambda kv: (-kv[1], kv[0]))[:lim]


def decode(inp, k=4, verbose=False):
    inp = inp.strip().lower()
    if not inp or not inp.isalpha():
        return []
    n = len(inp)
    # 双模式：输入能整串切成音节（纯全拼）→ 只允许音节边界上的跨度；
    # 否则（简拼/混拼/每字单字母）→ 字母网格。
    # 不做这个区分，全拼串会被 1 字母跨度拆成乱码
    # （实测 nihaoshijie → 你会啊哦时间而）。
    segs = segments(inp)
    bounds = None
    if segs:
        bounds = {0}
        pos = 0
        for sg in segs:
            pos += len(sg)
            bounds.add(pos)
    best = [[] for _ in range(n + 1)]
    best[0] = [(0.0, "", 0, [])]
    for i in range(1, n + 1):
        for j in range(max(0, i - MAX_SPAN), i):
            if not best[j]:
                continue
            if bounds is not None and (j not in bounds or i not in bounds):
                continue
            span = inp[j:i]
            if not (can_full(span) or mixed_viable(span)):
                continue
            hits = span_hits(span)
            if verbose and hits:
                print("    span %-12r -> %s" % (span, [(w, wt) for w, wt in hits[:3]]))
            wp = WORD_PENALTY if bounds is not None else WP_LETTER
            for w, wt in hits:
                step = __import__("math").log(max(wt, 1)) + LEN_BONUS * len(w) - wp
                if bounds is None and len(w) == len(span):
                    step += BONUS_INIT
                for sc, txt, nw, path in best[j]:
                    cand = (sc + step, txt + w, nw + 1, path + [(span, w)])
                    # 同文本去重必须**取更高分**（与 Rust push_beam 一致）。
                    # 之前保留先到者，导致 `你`+`好世界` 这种低分同形路径
                    # 把 `你好`+`世界` 挤掉（nihaoshijie → 你好师姐）。
                    same = next((c for c in best[i] if c[1] == cand[1]), None)
                    if same is not None:
                        if cand[0] > same[0]:
                            best[i].remove(same)
                            best[i].append(cand)
                        continue
                    best[i].append(cand)
        best[i].sort(key=lambda c: -c[0])
        del best[i][BEAM:]
    out = sorted(best[n], key=lambda c: -c[0])[:k]
    return [(txt, sc, nw, path) for sc, txt, nw, path in out]


CASES = [
    ("sj", "时间/世界(同权重,由词库定)", None),
    ("shij", "世界", None),
    ("wm", "我们", None),
    ("wmy", "我们没有 或 我们一", None),
    ("jttq", "今天天气/今天提前", None),
    ("jttqhh", "今天天气很好", None),
    ("nihaoshijie", "你好世界", None),
    ("woaibeijing", "我爱北京", None),
    ("wabj", "我爱北京", None),
    ("mingong", "民工", None),
    ("zhonghuarenmingongheguo", "中华人民共和国", None),
    ("zhrmghg", "中华人民共和国", None),
]

if __name__ == "__main__":
    verbose = "-v" in sys.argv
    for inp, note, _ in CASES:
        res = decode(inp, k=3, verbose=verbose)
        top = res[0] if res else None
        alt = " / ".join(r[0] for r in res[1:])
        print("%-26s -> %-18s  nwords=%d  top2-3: %-24s %s" % (
            inp, top[0] if top else "-", top[2] if top else 0, alt or "-", note))
        if top and verbose:
            print("      路径:", " | ".join("%s→%s" % (s, w) for s, w in top[3]))
