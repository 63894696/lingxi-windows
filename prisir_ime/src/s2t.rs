//! P2 简繁双向转换（2026-11-16，双向 2026-11-17）
//!
//! 数据来源：OpenCC（Apache-2.0）压成两份表：
//!   `data/s2t.txt`  STCharacters + STPhrases   简→繁  42,460 条 / 842KB
//!   `data/t2s.txt`  TSCharacters + TSPhrases   繁→简   3,578 条 /  32KB
//! 每行 `源<TAB>目标`，只保留首选项（OpenCC 同一 key 可给多个候选，第一个是推荐值）。
//!
//! 为什么必须带短语表：单字映射会在上下文上出错。
//!   简→繁 `鼠标里面`：逐字得到 鼠標里面（面 无映射）；短语表保证 鼠標裏面。
//!   繁→简 `乾乾淨淨`：逐字会得到 乾干净净；短语表保证 干干净净。
//!
//! 算法：从左到右扫描，每个位置优先尝试最长短语（上限 = 表内最长 key），
//! 命中则整体替换；否则查单字表；都不命中保持原字。

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

struct Table {
    char_map: HashMap<char, char>,
    phrase_map: HashMap<&'static str, &'static str>,
    /// 所有短语 key 的首字，用于快速跳过不可能命中短语的位置。
    phrase_start: HashSet<char>,
    max_key_chars: usize,
}

fn build(raw: &'static str) -> Table {
    let mut char_map: HashMap<char, char> = HashMap::new();
    let mut phrase_map: HashMap<&'static str, &'static str> = HashMap::new();
    let mut phrase_start: HashSet<char> = HashSet::new();
    let mut max_key_chars = 1usize;
    for line in raw.lines() {
        if line.is_empty() {
            continue;
        }
        let mut it = line.split('\t');
        let (Some(k), Some(v)) = (it.next(), it.next()) else { continue };
        let kc = k.chars().count();
        if kc == 0 {
            continue;
        }
        if kc == 1 {
            if let (Some(a), Some(b)) = (k.chars().next(), v.chars().next()) {
                char_map.insert(a, b);
            }
        } else {
            phrase_map.insert(k, v);
            if let Some(c) = k.chars().next() {
                phrase_start.insert(c);
            }
            if kc > max_key_chars {
                max_key_chars = kc;
            }
        }
    }
    Table { char_map, phrase_map, phrase_start, max_key_chars }
}

fn table_s2t() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(|| build(include_str!("../data/s2t.txt")))
}

fn table_t2s() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(|| build(include_str!("../data/t2s.txt")))
}

fn convert(t: &Table, input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(input.len() * 2);
    let mut i = 0;
    while i < n {
        // 只有首字可能起短语时才做最长匹配，否则直接走单字表。
        if t.phrase_start.contains(&chars[i]) {
            let max_len = t.max_key_chars.min(n - i);
            let mut len = max_len;
            let mut hit: Option<&'static str> = None;
            while len >= 2 {
                let s: String = chars[i..i + len].iter().collect();
                if let Some(v) = t.phrase_map.get(s.as_str()) {
                    hit = Some(v);
                    break;
                }
                len -= 1;
            }
            if let Some(v) = hit {
                out.push_str(v);
                i += len;
                continue;
            }
        }
        let c = chars[i];
        match t.char_map.get(&c) {
            Some(&v) => out.push(v),
            None => out.push(c),
        }
        i += 1;
    }
    out
}

/// 简体 → 繁体。
pub fn to_traditional(input: &str) -> String {
    convert(table_s2t(), input)
}

/// 繁体 → 简体。
pub fn to_simplified(input: &str) -> String {
    convert(table_t2s(), input)
}

/// 该串在某个方向上是否有可转换字符（供调用方"无变化就跳过"的快判）。
pub fn s2t_may_change(input: &str) -> bool {
    let t = table_s2t();
    input.chars().any(|c| t.char_map.contains_key(&c) || t.phrase_start.contains(&c))
}

pub fn t2s_may_change(input: &str) -> bool {
    let t = table_t2s();
    input.chars().any(|c| t.char_map.contains_key(&c) || t.phrase_start.contains(&c))
}

/// 表规模（诊断/测试）：(简→繁 单字, 简→繁 短语, 繁→简 单字, 繁→简 短语)
pub fn table_sizes() -> (usize, usize, usize, usize) {
    let a = table_s2t();
    let b = table_t2s();
    (a.char_map.len(), a.phrase_map.len(), b.char_map.len(), b.phrase_map.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_loaded() {
        let (s2t_c, s2t_p, t2s_c, t2s_p) = table_sizes();
        assert!(s2t_c > 3000, "简→繁单字表太小: {s2t_c}");
        assert!(s2t_p > 10000, "简→繁短语表太小: {s2t_p}");
        assert!(t2s_c > 2000, "繁→简单字表太小: {t2s_c}");
        assert!(t2s_p > 100, "繁→简短语表太小: {t2s_p}");
    }

    // ---------- 简 → 繁 ----------

    #[test]
    fn s2t_single_chars() {
        assert_eq!(to_traditional("国"), "國");
        assert_eq!(to_traditional("简"), "簡");
        assert_eq!(to_traditional("体"), "體");
    }

    #[test]
    fn s2t_sentences() {
        assert_eq!(to_traditional("计算机"), "計算機");
        assert_eq!(to_traditional("中华人民共和国"), "中華人民共和國");
        assert_eq!(to_traditional("软件"), "軟件");
    }

    #[test]
    fn s2t_phrase_beats_char() {
        // 逐字会把「面」留下（无映射），短语表保证整词转换
        assert_eq!(to_traditional("鼠标里面"), "鼠標裏面");
    }

    // ---------- 繁 → 简 ----------

    #[test]
    fn t2s_single_chars() {
        assert_eq!(to_simplified("國"), "国");
        assert_eq!(to_simplified("簡"), "简");
        assert_eq!(to_simplified("體"), "体");
    }

    #[test]
    fn t2s_sentences() {
        assert_eq!(to_simplified("計算機"), "计算机");
        assert_eq!(to_simplified("中華人民共和國"), "中华人民共和国");
        assert_eq!(to_simplified("軟件"), "软件");
    }

    #[test]
    fn t2s_phrase_beats_char() {
        // 乾乾淨淨：逐字会错（乾→干/幹 分不清），短语表保证 干干净净
        assert_eq!(to_simplified("乾乾淨淨"), "干干净净");
        // 一目瞭然：瞭 在「瞭然」里应作 了 而非 瞭/瞭
        assert_eq!(to_simplified("一目瞭然"), "一目了然");
    }

    #[test]
    fn roundtrip_common_text() {
        // 常见词往返应稳定（简→繁→简 回到原文）
        for s in ["计算机", "软件", "中华人民共和国", "北京", "输入法"] {
            let t = to_traditional(s);
            assert_eq!(to_simplified(&t), s, "{s} → {t} → ?");
        }
    }

    // ---------- 原样透传 ----------

    #[test]
    fn untouched_passthrough() {
        assert_eq!(to_traditional("繁體"), "繁體");
        assert_eq!(to_simplified("简体"), "简体");
        assert_eq!(to_traditional("hello 123"), "hello 123");
        assert_eq!(to_simplified("hello 123"), "hello 123");
        assert_eq!(to_traditional("，。！"), "，。！");
        assert_eq!(to_traditional(""), "");
        assert_eq!(to_simplified(""), "");
    }

    #[test]
    fn mixed_scripts() {
        assert_eq!(to_traditional("我用Python写代码"), "我用Python寫代碼");
        assert_eq!(to_simplified("我用Python寫代碼"), "我用Python写代码");
    }
}
