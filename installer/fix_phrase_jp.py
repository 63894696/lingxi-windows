# -*- coding: utf-8 -*-
"""Rebuild phrase.jp from key via first-letter jianpin; fix collisions like 单个 jp=de."""
from __future__ import annotations

import argparse
import sqlite3
import sys


def to_jp(key: str) -> str:
    """Naive jianpin: keep only a-z, take first letter of each syllable... 

    Our keys are concatenated full pinyin without separators (dange, zhongguo).
    True jianpin needs syllable segmentation. Heuristic used by lexicon:
    for known keys we re-derive from a simple vowel-boundary split is fragile.
    Instead: if existing jp looks like a full syllable equal to a prefix of key
    but key has more letters, recompute as first letters of greedy syllables.
    """
    # Use the same approach as many dumps: store first char of each pinyin syllable
    # by splitting with a minimal syllable table.
    from pathlib import Path

    # Prefer engine-compatible: take consecutive valid syllables.
    syll_path = Path(__file__).resolve().parents[1] / "prisir_ime" / "data"
    # Fall back: first letter of key only for 1-syllable; for multi use vowel groups.
    return _jianpin_greedy(key)


# Minimal legal syllable set (common pinyin) for greedy left-to-right.
_SYLL = None


def _load_syll() -> set[str]:
    global _SYLL
    if _SYLL is not None:
        return _SYLL
    # Compact list covering lexicon keys; include all initials+finals combos roughly
    # by reading pinyin table from DB if available — else builtin common set.
    common = """
    a ai an ang ao ba bai ban bang bao bei ben beng bi bian biao bie bin bing bo bu
    ca cai can cang cao ce cen ceng cha chai chan chang chao che chen cheng chi chong
    chou chu chua chuai chuan chuang chui chun chuo ci cong cou cu cuan cui cun cuo
    da dai dan dang dao de dei den deng di dia dian diao die ding diu dong dou du duan
    dui dun duo e ei en eng er fa fan fang fei fen feng fo fou fu
    ga gai gan gang gao ge gei gen geng gong gou gu gua guai guan guang gui gun guo
    ha hai han hang hao he hei hen heng hong hou hu hua huai huan huang hui hun huo
    ji jia jian jiang jiao jie jin jing jiong jiu ju juan jue jun
    ka kai kan kang kao ke ken keng kong kou ku kua kuai kuan kuang kui kun kuo
    la lai lan lang lao le lei leng li lia lian liang liao lie lin ling liu long lou
    lu luan lue lun luo lv
    ma mai man mang mao me mei men meng mi mian miao mie min ming miu mo mou mu
    na nai nan nang nao ne nei nen neng ni nian niang niao nie nin ning niu nong nou
    nu nuan nue nuo nv
    o ou pa pai pan pang pao pei pen peng pi pian piao pie pin ping po pou pu
    qi qia qian qiang qiao qie qin qing qiong qiu qu quan que qun
    ran rang rao re ren reng ri rong rou ru rua ruan rui run ruo
    sa sai san sang sao se sen seng sha shai shan shang shao she shei shen sheng shi
    shou shu shua shuai shuan shuang shui shun shuo si song sou su suan sui sun suo
    ta tai tan tang tao te teng ti tian tiao tie ting tong tou tu tuan tui tun tuo
    wa wai wan wang wei wen weng wo wu
    xi xia xian xiang xiao xie xin xing xiong xiu xu xuan xue xun
    ya yan yang yao ye yi yin ying yo yong you yu yuan yue yun
    za zai zan zang zao ze zei zen zeng zha zhai zhan zhang zhao zhe zhei zhen zheng
    zhi zhong zhou zhu zhua zhuai zhuan zhuang zhui zhun zhuo zi zong zou zu zuan
    zui zun zuo
    """.split()
    _SYLL = set(common)
    return _SYLL


def _jianpin_greedy(key: str) -> str:
    s = key.lower().strip()
    syll = _load_syll()
    out = []
    i = 0
    while i < len(s):
        matched = None
        for L in range(min(6, len(s) - i), 0, -1):
            piece = s[i : i + L]
            if piece in syll:
                matched = piece
                break
        if not matched:
            # skip one junk char
            i += 1
            continue
        out.append(matched[0])
        i += len(matched)
    return "".join(out) if out else (s[:1] if s else "")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", required=True)
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()
    if not a.apply and not a.dry_run:
        a.dry_run = True

    conn = sqlite3.connect(a.db)
    rows = conn.execute("SELECT key, value, jp, weight FROM phrase").fetchall()
    fixes = []
    for key, value, jp, w in rows:
        new_jp = _jianpin_greedy(key)
        if not new_jp:
            continue
        if (jp or "") != new_jp:
            fixes.append((key, value, jp, new_jp, w))

    print(f"[scan] phrase={len(rows):,} jp_mismatch={len(fixes):,}")
    # highlight dangerous: jp equals a full syllable but true jp differs
    danger = [f for f in fixes if f[2] and len(f[2]) <= 6 and f[2] != f[3] and f[2] in _load_syll()]
    print(f"[danger] jp-is-full-syllable-but-wrong: {len(danger):,}")
    for key, value, jp, new_jp, w in danger[:20]:
        print(f"  {value!r} key={key} jp={jp} -> {new_jp} w={w}")

    # always show 单个
    for key, value, jp, new_jp, w in fixes:
        if value == "单个":
            print(f"[check] 单个 key={key} jp={jp!r} -> {new_jp!r}")

    if a.dry_run:
        print("[dry-run] no changes")
        return 0

    conn.execute("BEGIN")
    n = 0
    for key, value, jp, new_jp, w in fixes:
        n += conn.execute(
            "UPDATE phrase SET jp=? WHERE key=? AND value=?",
            (new_jp, key, value),
        ).rowcount
    conn.commit()
    print(f"[apply] updated={n:,}")
    conn.execute("ANALYZE")
    conn.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
