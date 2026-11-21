# -*- coding: utf-8 -*-
"""候选词引擎 — 统一后端
处理拼音切分、候选词查询、排序
"""
import os
import time
from typing import List, Tuple, Optional
from .database import get_db
from .trie_index import get_memory_index

_LOG_FILE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "engine_learn.log")

def log(msg: str):
    try:
        with open(_LOG_FILE, "a", encoding="utf-8") as f:
            f.write(f"{time.strftime('%H:%M:%S')} {msg}\n")
    except Exception:
        pass


class CandidateEngine:
    """候选词引擎"""

    def __init__(self):
        self.db = get_db()
        self._pinyin_keys = None
        self._mem = get_memory_index()
        self._mem_started = False

    def _ensure_memory_index(self):
        """后台懒加载内存索引（只触发一次）。未就绪时查询仍走 SQLite。
        受 ime_config.ENABLE_MEMORY_INDEX 开关控制。"""
        if self._mem_started:
            return
        self._mem_started = True
        try:
            from . import ime_config
            if not ime_config.ENABLE_MEMORY_INDEX:
                return
            phrase = self.db.all_phrase_rows()
            pinyin = self.db.all_single_char_rows()
            self._mem.build_async(phrase, pinyin)
        except Exception:
            pass  # 构建失败不影响 SQLite 路径

    # 标准普通话音节表（约 400 个，不含声调）。词库 pinyin.key 被整词/长句污染，
    # 不能用来切分，故切分只用这张干净的单音节表。
    _SYLLABLES = frozenset("""
    a ai an ang ao
    ba bai ban bang bao bei ben beng bi bian biao bie bin bing bo bu
    ca cai can cang cao ce cen ceng cha chai chan chang chao che chen cheng chi chong chou chu chua chuai chuan chuang chui chun chuo ci cong cou cu cuan cui cun cuo
    da dai dan dang dao de dei den deng di dia dian diao die ding diu dong dou du duan dui dun duo
    e ei en eng er
    fa fan fang fei fen feng fo fou fu
    ga gai gan gang gao ge gei gen geng gong gou gu gua guai guan guang gui gun guo
    ha hai han hang hao he hei hen heng hong hou hu hua huai huan huang hui hun huo
    ji jia jian jiang jiao jie jin jing jiong jiu ju juan jue jun
    ka kai kan kang kao ke kei ken keng kong kou ku kua kuai kuan kuang kui kun kuo
    la lai lan lang lao le lei leng li lia lian liang liao lie lin ling liu long lou lu lv luan lve lun luo
    ma mai man mang mao me mei men meng mi mian miao mie min ming miu mo mou mu
    na nai nan nang nao ne nei nen neng ni nian niang niao nie nin ning niu nong nou nu nv nuan nve nun nuo
    o ou
    pa pai pan pang pao pei pen peng pi pian piao pie pin ping po pou pu
    qi qia qian qiang qiao qie qin qing qiong qiu qu quan que qun
    ran rang rao re ren reng ri rong rou ru ruan rui run ruo
    sa sai san sang sao se sen seng sha shai shan shang shao she shei shen sheng shi shou shu shua shuai shuan shuang shui shun shuo si song sou su suan sui sun suo
    ta tai tan tang tao te teng ti tian tiao tie ting tong tou tu tuan tui tun tuo
    wa wai wan wang wei wen weng wo wu
    xi xia xian xiang xiao xie xin xing xiong xiu xu xuan xue xun
    ya yan yang yao ye yi yin ying yo yong you yu yuan yue yun
    za zai zan zang zao ze zei zen zeng zha zhai zhan zhang zhao zhe zhei zhen zheng zhi zhong zhou zhu zhua zhuai zhuan zhuang zhui zhun zhuo zi zong zou zu zuan zui zun zuo
    """.split())

    def _get_pinyin_keys(self) -> List[str]:
        """切分用的音节表（按长度降序，保证 zh/ch/sh 优先于 z/c/s）。"""
        if self._pinyin_keys is None:
            self._pinyin_keys = sorted(self._SYLLABLES, key=len, reverse=True)
        return self._pinyin_keys

    def segment_pinyin(self, input_str: str) -> List[str]:
        """拼音切分：把连续拼音字符串切成有效拼音序列
        例: "nihao" -> ["ni", "hao"]
        """
        result = []
        i = 0
        keys = self._get_pinyin_keys()
        while i < len(input_str):
            matched = False
            for key in keys:
                if input_str[i:].startswith(key):
                    result.append(key)
                    i += len(key)
                    matched = True
                    break
            if not matched:
                # 无法匹配，跳过单字符
                i += 1
        return result

    def query(self, input_str: str, ime_type: str = "pinyin") -> List[Tuple[str, int]]:
        """查询候选词
        返回: [(候选词, 权重), ...]
        """
        if ime_type == "pinyin":
            return self._query_pinyin(input_str)
        elif ime_type == "wubi":
            return self._query_wubi(input_str)
        else:
            return []

    def _query_pinyin(self, input_str: str) -> List[Tuple[str, int]]:
        """拼音查询：单字 + 整词合并，按权重统一排序。

        旧逻辑是"phrase 有结果就直接返回、不查 pinyin"，导致生僻词组
        （如 mian→谜案、jie→饥饿）把高频单字（面/接）盖掉。改为合并去重。
        """
        self._ensure_memory_index()
        n_syll = len(self.segment_pinyin(input_str)) or 1
        seen = {}
        # 模糊音扩展（仅单音节单字查询，配置开启时）：扩展出等价拼音一起查
        fuzzy_inputs = [input_str]
        if n_syll == 1:
            fuzzy_inputs = self._fuzzy_expand(input_str)
        if self._mem.ready:
            # 内存索引路径：前缀查询一次拿全（单字+词组混合，已按权重排序）
            # 模糊音：对每个等价拼音做前缀查询合并（单音节时）
            for fp in fuzzy_inputs:
                for word, w in self._mem.query_prefix(fp, limit=50):
                    # 词组错配过滤：多字词的字数应等于音节数
                    if len(word) > 1 and len(word) != n_syll:
                        continue
                    # 黑名单过滤（内存索引不自动过滤，需在此处做）
                    if self.db.is_blacklisted(word):
                        continue
                    if word not in seen or w > seen[word]:
                        seen[word] = w
        else:
            # SQLite 路径（内存未就绪兜底；database.py 已过滤 blacklist）
            # 单字表（高频字：接/算/管/面/了…），模糊音对每个等价拼音查
            for fp in fuzzy_inputs:
                for word, w in self.db.query_pinyin(fp, limit=50):
                    if word not in seen or w > seen[word]:
                        seen[word] = w
            # 词组表（整词：你好/中国/鼠标…），过滤错配脏数据
            for word, w in self.db.query_phrase(input_str, limit=50):
                if len(word) != n_syll:
                    continue
                if word not in seen or w > seen[word]:
                    seen[word] = w
        # 混拼词组（正反向）：shij→世界、shij→中国、sjie→世界、zg→中国
        for word, w in self._query_phrase_mixed(input_str):
            if self.db.is_blacklisted(word):
                continue
            if word not in seen or w > seen[word]:
                seen[word] = w

        if seen:
            # 白名单加权：白名单中的词在同等权重下排更前
            def _sort_key(kv):
                word, weight = kv
                whitelist_bonus = 10_000_000 if self.db.is_whitelisted(word) else 0
                return (len(word) > 1, weight + whitelist_bonus)
            merged = sorted(seen.items(), key=_sort_key, reverse=True)
            return merged[:50]

        # 单字母（如 l）：查不到完整拼音，用首字母简拼带高频字（了/来/里…）
        if len(input_str) == 1:
            return self.db.query_pinyin_jp(input_str, limit=50)

        # 多字母无整匹配：切分后取首音节单字，保证中间态高频字仍可见
        segments = self.segment_pinyin(input_str)
        if segments:
            return self.db.query_pinyin(segments[0], limit=50)
        return []

    def _is_full_pinyin(self, s: str) -> bool:
        """s 能否完整切成音节（即整串都是全拼，无简拼首字母）。"""
        if not s:
            return False
        return bool(self.segment_pinyin(s)) and len("".join(self.segment_pinyin(s))) == len(s)

    # 模糊音映射：规则名 -> [(源, 目标)]。
    # 声母规则(z/c/s)在起始位置替换;韵母/鼻音规则在结尾位置替换(仅单音节应用)。
    _FUZZY_MAP = {
        # 声母(起始)
        "z_zh": [("zh", "z"), ("z", "zh")],
        "c_ch": [("ch", "c"), ("c", "ch")],
        "s_sh": [("sh", "s"), ("s", "sh")],
        # 前后鼻音(结尾): 前鼻音 -> 后鼻音 & 反向
        "an_ang": [("an", "ang"), ("ang", "an")],
        "en_eng": [("en", "eng"), ("eng", "en")],
        "in_ing": [("in", "ing"), ("ing", "in")],
        "ian_iang": [("ian", "iang"), ("iang", "ian")],
        "uan_uang": [("uan", "uang"), ("uang", "uan")],
        # 韵母(结尾)
        "ai_an": [("ai", "an"), ("an", "ai")],
        "un_ong": [("un", "ong"), ("ong", "un")],
    }
    # 声母规则(起始替换),其余按结尾替换
    _FUZZY_INITIAL = frozenset({"z_zh", "c_ch", "s_sh"})

    def _fuzzy_expand(self, inp: str) -> List[str]:
        """按 ime_config.FUZZY_RULES 扩展出等价拼音（含原音）。
        声母规则起始替换,韵母/鼻音规则结尾替换。替换结果须仍是合法单音节。"""
        try:
            from . import ime_config
            rules = ime_config.FUZZY_RULES
        except Exception:
            rules = frozenset()
        def _valid(cand):
            # 必须是合法单音节:切分恰好 1 段且该段覆盖整串(滤掉 zhhan/xiai 这类无效组合)
            segs = self.segment_pinyin(cand)
            return len(segs) == 1 and segs[0] == cand
        results = {inp}
        for rule in rules:
            for src, dst in self._FUZZY_MAP.get(rule, []):
                if rule in self._FUZZY_INITIAL:
                    # 起始替换(声母)
                    if inp.startswith(src):
                        cand = dst + inp[len(src):]
                        if _valid(cand):
                            results.add(cand)
                else:
                    # 结尾替换(韵母/鼻音)
                    if inp.endswith(src):
                        cand = inp[:len(inp) - len(src)] + dst
                        if _valid(cand):
                            results.add(cand)
        # 保持原音在前，其余任意顺序
        out = [inp] + [r for r in results if r != inp]
        return out

    def _query_phrase_mixed(self, inp: str) -> List[Tuple[str, int]]:
        """混拼词组查询（正反向），返回 [(词, 权重)]。

        只在输入不是"单音节全拼"时启用，避免干扰单字（jie→接 仍走单字）。
        四种命中：
          纯简拼  sj   -> 世界/时间     (jp = inp)
          前全拼  shij -> 世界/实际     (key LIKE inp%)
          后全拼  sjie -> 世界          (jp 首字母 + key 以 jie 结尾)
        """
        if len(inp) < 2:
            return []
        # 单音节全拼（jie/suan/mian）不混拼，交给单字逻辑，避免词组抢单字
        segs = self.segment_pinyin(inp)
        if self._is_full_pinyin(inp) and len(segs) == 1:
            return []
        seen = {}
        def add(rows):
            for val, w in rows:
                if len(val) < 2:      # 混拼只要词组，不要单字
                    continue
                if val not in seen or w > seen[val]:
                    seen[val] = w
        # 纯简拼（整串非全拼）：sj / zg / wm
        if not self._is_full_pinyin(inp):
            add(self.db.query_phrase_by_jp(inp, limit=20))
            # 反向混拼：首字母 + 后续全拼（sjie→世界）
            rest = inp[1:]
            if self._is_full_pinyin(rest):
                add(self.db.query_phrase_reverse_mixed(inp[0], rest, limit=15))
        # 前全拼 + 后首字母：key 前缀（shij→世界，zhongg→中国）
        add(self.db.query_phrase_by_key_prefix(inp, limit=15))
        return sorted(seen.items(), key=lambda kv: kv[1], reverse=True)[:15]

    def smart_sentence(self, input_str: str) -> Optional[str]:
        """整句智能首选：用词组做最优路径分词（DP/Viterbi），拼出最可能整句。

        在拼音音节流上找一条"尽量用高权词组"的路径。词组优先（每用一个 >=2 音节
        词组给固定奖励），单字兜底。返回拼接整句；无法切分或无路径返回 None。
        """
        import math
        segs = self.segment_pinyin(input_str)
        n = len(segs)
        if n < 2:
            return None

        def best_word(pseq):
            key = "".join(pseq)
            rows = self.db.query_phrase(key, limit=1)
            if rows:
                return rows[0]
            if len(pseq) == 1:
                r = self.db.query_pinyin(key, limit=5)
                singles = [(w, wt) for w, wt in r if len(w) == 1]
                if singles:
                    return singles[0]
            return None

        # 优先:整句若命中覆盖全部音节的完整整词(如 shurufa->输入法),直接返回。
        # 否则 DP 会让"高频词组+单字"的组合(输入+发)挤掉真正的整词(输入法)。
        full = best_word(segs)
        if full and len(full[0]) == n:
            return full[0]

        NEG = float("-inf")
        dp = [(NEG, []) for _ in range(n + 1)]
        dp[0] = (0.0, [])
        for i in range(1, n + 1):
            for j in range(max(0, i - 4), i):  # 词最长 4 音节
                pseq = segs[j:i]
                bw = best_word(pseq)
                if not bw or dp[j][0] == NEG:
                    continue
                word, weight = bw
                bonus = 20.0 if len(pseq) >= 2 else 0.0
                score = dp[j][0] + math.log(max(weight, 1)) + bonus
                if score > dp[i][0]:
                    dp[i] = (score, dp[j][1] + [word])
        if dp[n][0] == NEG or not dp[n][1]:
            return None
        return "".join(dp[n][1])

    def _query_wubi(self, input_str: str) -> List[Tuple[str, int]]:
        """五笔查询（预留）"""
        return self.db.query_wubi(input_str, limit=10)

    def learn(self, input_str: str, selected_word: str):
        """学习用户选择：只做词频学习，不再自动写单字母别名。

        为什么移除自动别名（2026-11-16 设计纠偏）：
          「前/后一个字用首字母代替」这一输入习惯，混拼已原生支持：
            sjie -> query_phrase_reverse_mixed   （首字母 + 余下全拼）
            shij -> query_phrase_by_key_prefix   （前全拼 + 末首字母）
            sj   -> query_phrase_by_jp           （纯简拼）
          再额外把 's'/'j' 写成 phrase 表的 alias 行，会造成：
            1) 单字母候选被历史选过的所有词污染；
            2) phrase 表行数失控；
            3) 与 jp（正确简拼列）语义冲突。
        高频词快速命中的正确机制是「权重自学习」，由 add_user_word 承担（Rust 侧为
        user.db + LEARN_PIN_WEIGHT 置顶且位置锁定）。
        """
        if not input_str or not selected_word:
            return
        # 权重学习：多字词写 phrase 表并累加权重，单字才更新 pinyin 表（database 内部已分流）
        self.db.add_user_word(input_str, selected_word, weight=1)


# 全局单例
_engine = None

def get_engine() -> CandidateEngine:
    global _engine
    if _engine is None:
        _engine = CandidateEngine()
    return _engine
