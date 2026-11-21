//! P1 整句解码：跨度网格 + 束搜索（2026-11-16）
//!
//! ## 为什么需要它
//! 旧 `smart_sentence` 先用 `segment()` 切音节、再按音节段查词库。这对**全拼**有效，
//! 但简拼/混拼根本切不出音节（`sj` / `wabj` / `zhrmghg` 的 `segment()` 是空或垃圾），
//! 于是整句能力对“每字单字母”完全失效。实测基线 13 条用例 8 条 MISS。
//!
//! ## 做法：直接在字节跨度上查库
//! 跨度 `input[j..i]` 的原样子串天然覆盖三种输入形态：
//!   全拼 `jintian` → `phrase.key` 精确
//!   简拼 `jt`      → `phrase.jp`  精确（jt → 今天）
//!   混拼 `shij`    → `phrase.key` 前缀（shij → shijie → 世界）
//!   单字母 `w`     → `pinyin.jp`  精确（单字按首字母 → 我）
//! 于是全拼 / 简拼 / 混拼 / 每字单字母共用**一套**解码。
//!
//! ## 双模式（关键）
//! - 输入能整串切成音节（纯全拼）→ **只允许落在音节边界上的跨度**。
//!   不这么做，全拼串会被 1 字母跨度拆成乱码（实测 `nihaoshijie` → `你会啊哦时间而`）。
//! - 否则（简拼/混拼/每字单字母）→ **字母网格**，每个字母都可起跨。
//!
//! ## 约束：词字数 ≤ 跨度承载能力
//! 全拼跨度 = 音节数（`shi` 只能出单字）；简拼跨度 = 字母数。
//! 没有这条，`shi` 会通过 key 前缀匹出「实现」这种 2 字词，1 个音节吃 3 个字节的语义，
//! 整句立刻崩（实测 `nihaoshijie` → `你和你阿牛哦实际而`）。
//!
//! ## 打分（实测调参）
//! 词库权重已归一化（词组普遍 ~5e5、单字 ~2.5e4），`ln(weight)` 几乎拉不开差距，
//! 而且**逐词累加 ln(weight) 会奖励“多切几刀”**（切 3 刀就拿 3 份 ln(5e5)）。
//! 真正起作用的是「词数惩罚」：
//!   - 音节模式 `WORD_PENALTY_SYLL = 18`（> ln(5e5)≈13.1，保证长词胜过多词）
//!   - 字母模式 `WORD_PENALTY_LETTER = 12`（轻一些，否则“每字单字母”路径被压死，
//!     实测 `wabj` → 哇北京，而正确的 我爱北京 根本排不上）
//!
//! ## 束搜索
//! 每个位置保留 top-`BEAM` 条假设，避免早期次优切分被永久锁死（旧 DP 只留 1 条）。
//! **同文本去重取高分**：`你`+`好世界` 与 `你好`+`世界` 文本相同但分数差很多，
//! 保留先到者会让正确解被挤掉（实测 `nihaoshijie` → `你好师姐`）。

use crate::engine::ImeEngine;
use std::collections::HashMap;

/// 单条假设最多覆盖多少字节。
const MAX_SPAN: usize = 24;
/// 每个位置保留的假设数。
const BEAM: usize = 4;
/// 每覆盖 1 个汉字给的奖励。
const LEN_BONUS: f64 = 1.5;
/// 音节模式（纯全拼）每切一刀的惩罚。必须 > ln(典型词权重)≈13.1。
const WORD_PENALTY_SYLL: f64 = 18.0;
/// 字母模式（简拼/混拼/每字单字母）每切一刀的惩罚。
/// 必须 > ln(典型词权重)≈13.1 才能让"整串命中一个长词"胜过多词拆分
/// （实测 15：`zhrmghg` → 中华人民共和国）；但压太死会杀掉单字母路径，
/// 故用 `BONUS_INITIAL` 给单字母路径补回来。
const WORD_PENALTY_LETTER: f64 = 15.0;
/// 每字单字母奖励：字母模式下，词字数 == 跨度字母数（一字母一汉字）额外加分。
/// 这是"每字单字母快速出字"的显式信号，没有它 `wabj` 会出 `哇北京`
/// 而不是 `我啊北京`（`我爱北京` 靠 top-K 候选选）。
const BONUS_INITIAL: f64 = 2.0;
/// 长跨度最多取多少个候选词。
const SPAN_LIMIT: usize = 6;
/// 短跨度（≤2 字母）放宽到这个数：短跨度候选权重几乎持平
/// （`中华` 在 jp=zh 里只排第 9），卡 6 会让正确路径不可达。
const SPAN_LIMIT_SHORT: usize = 12;
/// key 前缀匹配的最小跨度长度。前缀只服务混拼（shij → shijie），
/// 跨度太短会命中海量词（实测 `zh` → 制度/知道）把整句带偏。
const MIN_PREFIX_SPAN: usize = 4;

#[derive(Clone, Debug)]
struct Hyp {
    score: f64,
    text: String,
    nwords: usize,
}

/// 一条解码结果。`nwords` 让调用方区分“整句”还是“单个词”。
#[derive(Clone, Debug)]
pub struct Decoded {
    pub sentence: String,
    pub score: f64,
    pub nwords: usize,
}

impl ImeEngine {
    /// 整句解码，返回 top-`k`。输入非法（空/含非字母）返回空。
    pub fn decode(&self, input: &str, k: usize) -> Vec<Decoded> {
        let inp = input.trim().to_ascii_lowercase();
        if inp.is_empty() || !inp.bytes().all(|b| b.is_ascii_alphabetic()) {
            return Vec::new();
        }
        let n = inp.len();
        let k = k.max(1);

        // 双模式：能整串切音节 → 只在音节边界取跨度；否则字母网格。
        let bounds: Option<Vec<bool>> = self.syll.segments(&inp).map(|segs| {
            let mut b = vec![false; n + 1];
            b[0] = true;
            let mut pos = 0;
            for sg in segs {
                pos += sg.len();
                if pos <= n {
                    b[pos] = true;
                }
            }
            b
        });
        let wp = if bounds.is_some() { WORD_PENALTY_SYLL } else { WORD_PENALTY_LETTER };

        let mut best: Vec<Vec<Hyp>> = vec![Vec::new(); n + 1];
        best[0].push(Hyp { score: 0.0, text: String::new(), nwords: 0 });
        let mut memo: HashMap<(usize, usize), Vec<(String, i64)>> = HashMap::new();

        for i in 1..=n {
            let lo = i.saturating_sub(MAX_SPAN);
            for j in lo..i {
                if best[j].is_empty() {
                    continue;
                }
                if let Some(bs) = &bounds {
                    if !bs[j] || !bs[i] {
                        continue;
                    }
                }
                let span = &inp[j..i];
                if !(self.syll.can_full_segment(span) || self.syll.mixed_viable(span)) {
                    continue;
                }
                let hits = match memo.get(&(j, i)) {
                    Some(v) => v.clone(),
                    None => {
                        let v = self.span_hits(span);
                        memo.insert((j, i), v.clone());
                        v
                    }
                };
                if hits.is_empty() {
                    continue;
                }
                let prev = best[j].clone();
                for (word, weight) in &hits {
                    let len = word.chars().count().max(1);
                    let mut step = ((*weight).max(1) as f64).ln() + LEN_BONUS * len as f64 - wp;
                    // 一字母一汉字（每字单字母输入）额外奖励，仅字母模式给。
                    if bounds.is_none() && len == span.len() {
                        step += BONUS_INITIAL;
                    }
                    for h in &prev {
                        push_beam(&mut best[i], Hyp {
                            score: h.score + step,
                            text: format!("{}{}", h.text, word),
                            nwords: h.nwords + 1,
                        });
                    }
                }
            }
        }

        let mut out: Vec<Decoded> = best[n]
            .iter()
            .map(|h| Decoded { sentence: h.text.clone(), score: h.score, nwords: h.nwords })
            .collect();
        out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        out.truncate(k);
        out
    }

    /// 单个跨度（原样子串）能命中的词。**不做模糊扩张** —— 模糊音属于候选层，
    /// 放进整句解码会让每个跨度都炸出无关候选。
    fn span_hits(&self, span: &str) -> Vec<(String, i64)> {
        let max_chars = self.max_word_chars(span);
        let lim = if span.len() <= 2 { SPAN_LIMIT_SHORT } else { SPAN_LIMIT };
        let mut map: HashMap<String, i64> = HashMap::new();
        let mut put = |w: String, wt: i64| {
            if w.is_empty() || w.chars().count() > max_chars {
                return;
            }
            map.entry(w).and_modify(|e| *e = (*e).max(wt)).or_insert(wt);
        };

        // 1) 全拼/混拼整词：phrase.key 精确
        for (w, wt) in self.db.query_phrase(span, lim).unwrap_or_default() {
            put(w, wt);
        }
        // 2) 简拼整词：phrase.jp 精确（jt → 今天）
        for (w, wt) in self.db.query_phrase_by_jp(span, lim).unwrap_or_default() {
            put(w, wt);
        }
        // 3) 混拼：key 前缀（shij → shijie → 世界）
        if span.len() >= MIN_PREFIX_SPAN {
            for (w, wt) in self.db.query_phrase_by_key_prefix(span, SPAN_LIMIT).unwrap_or_default() {
                put(w, wt);
            }
        }
        // 4) 单字：全拼（jin → 进）与单字母简拼（j → 就/几/…）
        let chars = if span.len() == 1 {
            self.db.query_pinyin_jp(span, lim).unwrap_or_default()
        } else if self.syll.can_full_segment(span) {
            self.db.query_pinyin(span, lim).unwrap_or_default()
        } else {
            Vec::new()
        };
        for (w, wt) in chars {
            if w.chars().count() == 1 {
                put(w, wt);
            }
        }

        let mut out: Vec<(String, i64)> = map.into_iter().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out.truncate(lim);
        out
    }

    /// 一个跨度最多能承载几个汉字。
    /// 全拼跨度 = 音节数；简拼/混拼跨度 = 字母数（每个字母至少一个汉字）。
    fn max_word_chars(&self, span: &str) -> usize {
        match self.syll.segments(span) {
            Some(segs) => segs.len().max(1),
            None => span.len().max(1),
        }
    }
}

/// 把假设插进该位置的束：同文本取高分，按分数降序保留前 `BEAM` 条。
fn push_beam(beam: &mut Vec<Hyp>, cand: Hyp) {
    if let Some(slot) = beam.iter_mut().find(|h| h.text == cand.text) {
        if cand.score > slot.score {
            *slot = cand;
        }
    } else {
        beam.push(cand);
    }
    beam.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    beam.truncate(BEAM);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beam_keeps_top_and_dedupes_by_higher_score() {
        let mut b = Vec::new();
        push_beam(&mut b, Hyp { score: 1.0, text: "a".into(), nwords: 1 });
        push_beam(&mut b, Hyp { score: 3.0, text: "b".into(), nwords: 1 });
        push_beam(&mut b, Hyp { score: 2.0, text: "c".into(), nwords: 1 });
        assert_eq!(b[0].text, "b");
        assert_eq!(b.len(), 3);
        // 同文本必须取更高分（这条曾是 Python 原型的坑，导致首选被次优路径挤掉）
        push_beam(&mut b, Hyp { score: 9.0, text: "a".into(), nwords: 1 });
        assert_eq!(b[0].text, "a");
        assert_eq!(b[0].score, 9.0);
        assert_eq!(b.len(), 3);
        // 更低分不得覆盖
        push_beam(&mut b, Hyp { score: 0.1, text: "a".into(), nwords: 1 });
        assert_eq!(b[0].score, 9.0);
        // 超出 BEAM 被截断
        for i in 0..10 {
            push_beam(&mut b, Hyp { score: 100.0 + i as f64, text: format!("z{i}"), nwords: 1 });
        }
        assert_eq!(b.len(), BEAM);
    }
}
