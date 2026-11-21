//! P0 中英文自动判定（2026-11-16）
//!
//! 背景：豆包 obric-ime-engine 用 `MultiFrameInput` +
//! `ShouldPreferTrustedEnglishOnScriptSwitch` / `ShouldPreferConfidentEnglishOnScriptSwitch`
//! 做语种切换判定。那是多帧模型 + 置信度，成本高。
//!
//! 这里用**两级判定**达到同样目的，无需模型：
//!   1. 音节表可切分性 —— 整串能否切成合法普通话（全拼 / 简拼 / 混拼）；
//!   2. 词库命中 —— 可切分但词库一条都查不到，说明是英文单词而非拼音。
//!
//! 判定结果只影响"这一笔输入怎么上屏"，不改全局中/英模式。

use crate::syllable::Syllables;

/// 语种判定结果。数值与 C ABI `prisir_ime_classify` 的返回值一一对应。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum LangKind {
    /// 缓冲为空
    Empty = 0,
    /// 拼音（含全拼/简拼/混拼）
    Pinyin = 1,
    /// 英文（按原始字母上屏）
    English = 2,
    /// 单字母等无法判定，保持拼音行为
    Ambiguous = 3,
}

impl LangKind {
    pub fn as_u32(self) -> u32 {
        self as u32
    }

    /// 是否应当把原始字母当英文上屏。
    pub fn is_english(self) -> bool {
        matches!(self, LangKind::English)
    }
}

/// 判定策略（纯函数，便于单测）。
///
/// - 空 → Empty
/// - 含非 ASCII 字母 → Pinyin（不属于英文直通场景，交回原逻辑）
/// - 单字母 → Ambiguous（保持现有单字母候选行为，避免打断输入节奏）
/// - 能整串切成全拼 → Pinyin（即使词库暂时没命中，用户可能还在输入前缀）
/// - 能切成简拼/混拼：词库有**精确命中** → Pinyin；无 → English
/// - 连简拼都切不动 → English
///
/// 注意：判据用的必须是“精确命中”(phrase.key == input 或 jp == input)，
/// 不能用“候选数 > 0”。引擎的 `query()` 有模糊音/贪心切分兼底，
/// 任何字母串都能返回一堆候选（`hello` 实测 50 条），用它判会全判成拼音。
pub fn decide(syll: &Syllables, input: &str, has_exact_hit: bool) -> LangKind {
    if input.is_empty() {
        return LangKind::Empty;
    }
    if !input.bytes().all(|b| b.is_ascii_alphabetic()) {
        return LangKind::Pinyin;
    }
    let lower = input.to_ascii_lowercase();
    let s = lower.as_str();
    if s.chars().count() == 1 {
        return LangKind::Ambiguous;
    }
    if syll.can_full_segment(s) {
        return LangKind::Pinyin;
    }
    if syll.mixed_viable(s) {
        return if has_exact_hit { LangKind::Pinyin } else { LangKind::English };
    }
    LangKind::English
}

#[cfg(test)]
mod tests {
    use super::*;

    fn syl() -> Syllables {
        Syllables::new()
    }

    #[test]
    fn empty_and_single_letter() {
        let s = syl();
        assert_eq!(decide(&s, "", false), LangKind::Empty);
        assert_eq!(decide(&s, "s", false), LangKind::Ambiguous);
        assert_eq!(decide(&s, "z", true), LangKind::Ambiguous);
    }

    #[test]
    fn full_pinyin_stays_pinyin() {
        let s = syl();
        // 即使词库没命中，全拼也按拼音处理（用户可能还在输入前缀）
        assert_eq!(decide(&s, "nihao", false), LangKind::Pinyin);
        assert_eq!(decide(&s, "shijie", false), LangKind::Pinyin);
        assert_eq!(decide(&s, "mingong", false), LangKind::Pinyin);
    }

    #[test]
    fn mixed_pinyin_needs_exact_hit() {
        let s = syl();
        // 混拼 sjie / shij：可切分，词库有精确命中 → 拼音
        assert_eq!(decide(&s, "sjie", true), LangKind::Pinyin);
        assert_eq!(decide(&s, "shij", true), LangKind::Pinyin);
        // 同样可切分但词库无精确命中 → 判英文
        assert_eq!(decide(&s, "sjie", false), LangKind::English);
    }

    #[test]
    fn english_words_are_english() {
        let s = syl();
        assert_eq!(decide(&s, "hello", false), LangKind::English);
        assert_eq!(decide(&s, "world", false), LangKind::English);
        assert_eq!(decide(&s, "xyz", false), LangKind::English);
        assert_eq!(decide(&s, "ok", false), LangKind::English);
    }

    #[test]
    fn non_alpha_is_not_english() {
        let s = syl();
        assert_eq!(decide(&s, "ni3", false), LangKind::Pinyin);
    }

    #[test]
    fn uppercase_folds_to_lower() {
        let s = syl();
        assert_eq!(decide(&s, "NIHAO", false), LangKind::Pinyin);
    }
}
