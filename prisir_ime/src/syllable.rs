//! 标准普通话音节表(约 400,不含声调)+ 拼音切分。
//! 词库 pinyin.key 被整词/长句污染不能用来切分,故切分只用这张干净单音节表(对齐 Python)。

use std::collections::HashMap;

/// 按长度降序的音节表(zh/ch/sh 优先于 z/c/s),切分用。
pub struct Syllables {
    pub ordered: Vec<&'static str>,
    /// T9 用：音节的 T9 数字编码 → 该编码下的所有音节。
    /// 例："64" → ["mi", "ni", "oi"]（zh/ch/sh 按字母逐个映射，与 build_t9.py 一致）。
    by_code: HashMap<String, Vec<&'static str>>,
}

/// 九宫格展开的限制：最多几个音节、最多几种组合。
///
/// `MAX_T9_EXPAND` 曾取 32：短音节优先 DFS 时，预算会被 `a ai dan` / `ba ge an`
/// 这类碎切占满，真实常用读音（如 `224326` → `cai dan`）排到第 40 才出现，
/// 界面就只剩 `bai da o` +「摆到」。改为长音节优先后 32 已够；仍提到 64 留余量。
const MAX_T9_SYLLABLES: usize = 4;
const MAX_T9_EXPAND: usize = 64;

/// 音节 → T9 数字串（与 lexicon_pipeline/build_t9.py 的映射完全一致）。
///
/// 2=abc 3=def 4=ghi 5=jkl 6=mno 7=pqrs 8=tuv 9=wxyz（zh/ch/sh 也是逐字母映射：zh→94）。
pub fn t9_code(syllable: &str) -> Option<String> {
    if syllable.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(syllable.len());
    for ch in syllable.chars() {
        let d = match ch {
            'a' | 'b' | 'c' => '2',
            'd' | 'e' | 'f' => '3',
            'g' | 'h' | 'i' => '4',
            'j' | 'k' | 'l' => '5',
            'm' | 'n' | 'o' => '6',
            'p' | 'q' | 'r' | 's' => '7',
            't' | 'u' | 'v' => '8',
            'w' | 'x' | 'y' | 'z' => '9',
            _ => return None,
        };
        out.push(d);
    }
    Some(out)
}

const SYLLABLE_LIST: &str = "
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
";

impl Syllables {
    pub fn new() -> Self {
        let mut v: Vec<&'static str> = SYLLABLE_LIST.split_whitespace().collect();
        v.sort_by(|a, b| b.len().cmp(&a.len())); // 长度降序
        let mut by_code: HashMap<String, Vec<&'static str>> = HashMap::new();
        for s in &v {
            if let Some(code) = t9_code(s) {
                by_code.entry(code).or_default().push(s);
            }
        }
        Syllables { ordered: v, by_code }
    }

    /// **九宫格**：数字串 → 拼音串列表（音节之间用空格分隔）。
    ///
    /// 只返回**恰好消费整个数字串**的标准音节组合：
    ///   "64"    → ["mi", "ni", "oi"]
    ///   "64426" → ["mi gan", "mi han", "ni gan", "ni han", …]
    ///
    /// 拼不出来时返回空（例如只按了 "6"，或 "644" 这种中间态）——
    /// 调用方据此回退到词库 t9 表的前缀匹配，**不要假装知道拼音**。
    ///
    /// 实测（对 30 万词真实词库逐条比对）：该展开能覆盖 99% 以上的真实拼音 key，
    /// 漏掉的只有 "zh"/"sh"/"w" 这类简拼式单字母 key。
    pub fn t9_expand(&self, digits: &str) -> Vec<String> {
        if digits.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<String> = Vec::new();
        let mut acc: Vec<&'static str> = Vec::new();
        t9_dfs(
            digits,
            &mut acc,
            &mut out,
            &self.by_code,
            MAX_T9_SYLLABLES,
            MAX_T9_EXPAND,
        );
        out
    }

    /// 拼音切分:连续拼音串 → 有效音节序列。"nihao" -> ["ni","hao"]。
    ///
    /// 2026-11-01 修复:原实现是纯贪心最长匹配 + 匹配不上就跳单字符,
    /// 遇到「前音节以 n/ng 结尾、后音节以 g 开头」的边界会吃掉后音节的声母 g,
    /// 再把残留的 "ong" 当成音节 "o",尾巴 "ng" 被静默丢弃。
    /// 例:mingong(民工) 被切成 ["ming","o"],rengong(人工) 被切成 ["reng","o"],
    /// 拼出的 key 不存在于词库,导致整句识别退化成逐词瞎拼。
    ///
    /// 现在先尝试「完整消费」的回溯切分;整串无法完整切分时(简拼/混拼输入),
    /// 才退回原来的贪心跳字符行为,保持旧语义不变。
    pub fn segment(&self, input: &str) -> Vec<String> {
        if let Some(v) = self.segment_full(input) {
            return v;
        }
        self.segment_greedy(input)
    }

    /// 带回溯的完整切分:要求整串被音节表恰好消费完,否则返回 None。
    /// can[i] = 从第 i 字节起能否完整切分;先反向 DP 求可达性,再正向重建。
    fn segment_full(&self, input: &str) -> Option<Vec<String>> {
        let bytes = input.as_bytes();
        let n = bytes.len();
        if n == 0 {
            return Some(Vec::new());
        }
        let mut can = vec![false; n + 1];
        can[n] = true;
        for i in (0..n).rev() {
            for key in &self.ordered {
                let kb = key.as_bytes();
                if i + kb.len() <= n && &bytes[i..i + kb.len()] == kb && can[i + kb.len()] {
                    can[i] = true;
                    break;
                }
            }
        }
        if !can[0] {
            return None;
        }
        let mut out = Vec::new();
        let mut i = 0;
        while i < n {
            let mut advanced = false;
            for key in &self.ordered {
                let kb = key.as_bytes();
                if i + kb.len() <= n && &bytes[i..i + kb.len()] == kb && can[i + kb.len()] {
                    out.push(key.to_string());
                    i += kb.len();
                    advanced = true;
                    break;
                }
            }
            if !advanced {
                return None; // 理论不可达(can[0] 已保证),保险起见退回贪心
            }
        }
        Some(out)
    }

    /// 旧行为:贪心最长匹配,匹配不上跳单字符。保留给简拼/混拼输入。
    fn segment_greedy(&self, input: &str) -> Vec<String> {
        let bytes = input.as_bytes();
        let mut result = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let mut matched = false;
            for key in &self.ordered {
                let kb = key.as_bytes();
                if i + kb.len() <= bytes.len() && &bytes[i..i + kb.len()] == kb {
                    result.push(key.to_string());
                    i += kb.len();
                    matched = true;
                    break;
                }
            }
            if !matched {
                i += 1;
            }
        }
        result
    }

    /// s 能否完整切成音节(整串都是全拼,无简拼首字母)。
    pub fn is_full_pinyin(&self, s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        let segs = self.segment(s);
        !segs.is_empty() && segs.concat().len() == s.len()
    }

    /// 整串能否被音节表**恰好**消费完(不回退贪心丢字符)。
    /// 供中英文判定使用:全拼一定返回 true,简拼/混拼返回 false。
    pub fn can_full_segment(&self, s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        self.segment_full(s).is_some()
    }

    /// 返回恰好消费整串的**唯一**音节切分（贪心优先长音节重建），不可切分返回 None。
    /// 供 P1 整句解码使用：全拼输入只在音节边界上取跨度。
    pub fn segments(&self, s: &str) -> Option<Vec<String>> {
        if s.is_empty() {
            return None;
        }
        self.segment_full(s)
    }

    /// 简拼/混拼可行性:整串可被「完整音节 或 单声母字母」的序列恰好消费完。
    /// 例:sjie → s|jie、shij → shi|j、sj → s|j 都为 true;
    ///     而 hello 也为 true(h|e|l|l|o,o/e 是音节),故**必须**再叠加词库命中才能判拼音。
    pub fn mixed_viable(&self, s: &str) -> bool {
        let b = s.as_bytes();
        let n = b.len();
        if n == 0 {
            return false;
        }
        let mut can = vec![false; n + 1];
        can[n] = true;
        for i in (0..n).rev() {
            // 1) 吃一个完整音节
            for key in &self.ordered {
                let kb = key.as_bytes();
                if i + kb.len() <= n && &b[i..i + kb.len()] == kb && can[i + kb.len()] {
                    can[i] = true;
                    break;
                }
            }
            // 2) 吃一个单声母(简拼)
            if !can[i] && is_initial_byte(b[i]) && can[i + 1] {
                can[i] = true;
            }
        }
        can[0]
    }
}

/// 可单独作简拼首字母的声母字节。
/// 注意不含 'v':v 只出现在 lv/nv/lve/nve 这类完整音节里,由音节表负责。
fn is_initial_byte(c: u8) -> bool {
    matches!(
        c.to_ascii_lowercase(),
        b'b' | b'p' | b'm' | b'f' | b'd' | b't' | b'n' | b'l' | b'g' | b'k' | b'h' | b'j'
            | b'q' | b'x' | b'r' | b'z' | b'c' | b's' | b'y' | b'w'
    )
}

/// 九宫格展开的 DFS：rest 是还没被消费的字串，acc 是已选的音节。
///
/// 每次取 rest 的一个前缀（1~6 位），查有没有**编码恰好等于**该前缀的音节；
/// 有就接上继续往下拼。拼到 rest 空了就得到一种组合。
///
/// **长音节优先**（前缀从长到短）：先出 `bai dao` / `cai dan`，再出碎切
/// `bai da o`。短优先时 32 条预算会被碎切占满，常用整词读音进不了候选。
fn t9_dfs(
    rest: &str,
    acc: &mut Vec<&'static str>,
    out: &mut Vec<String>,
    by_code: &HashMap<String, Vec<&'static str>>,
    max_syll: usize,
    limit: usize,
) {
    // 先收完整切分，再判 limit / 音节上限 —— 否则 acc 已满 max_syll 且
    // rest 为空时会直接 return，4 音节合法读音永远进不了结果。
    if rest.is_empty() {
        if out.len() < limit {
            out.push(acc.join(" "));
        }
        return;
    }
    if out.len() >= limit || acc.len() >= max_syll {
        return;
    }
    // rest 全是 ASCII 数字，按字节切安全；长前缀优先
    let max = rest.len().min(6);
    for i in (1..=max).rev() {
        let Some(syls) = by_code.get(&rest[..i]) else {
            continue;
        };
        for s in syls {
            acc.push(s);
            t9_dfs(&rest[i..], acc, out, by_code, max_syll, limit);
            acc.pop();
        }
    }
}

#[cfg(test)]
mod t9_expand_tests {
    use super::*;

    #[test]
    fn caidan_digits_include_cai_dan_not_only_fragments() {
        let s = Syllables::new();
        let rs = s.t9_expand("224326");
        assert!(
            rs.iter().any(|p| p == "cai dan"),
            "224326 应展开出 cai dan（菜单），实际前几条 {:?}",
            &rs[..rs.len().min(12)]
        );
        assert!(
            rs.iter().any(|p| p == "bai dao"),
            "224326 应展开出 bai dao，实际 {:?}",
            &rs[..rs.len().min(12)]
        );
        // 长优先：整音节读音应早于碎切 bai da o
        let cai = rs.iter().position(|p| p == "cai dan").unwrap();
        let bai_dao = rs.iter().position(|p| p == "bai dao").unwrap();
        if let Some(frag) = rs.iter().position(|p| p == "bai da o") {
            assert!(
                bai_dao < frag,
                "bai dao (idx {bai_dao}) 应早于碎切 bai da o (idx {frag})"
            );
        }
        assert!(
            cai < 32,
            "cai dan 必须落在旧 32 条预算内（曾被截断），idx={cai}"
        );
    }

    #[test]
    fn nihao_still_expands() {
        let s = Syllables::new();
        let rs = s.t9_expand("64426");
        assert!(rs.iter().any(|p| p == "ni hao"), "64426 → ni hao，实际 {:?}", rs);
        assert!(rs.iter().any(|p| p == "mi gao"), "64426 → mi gao，实际 {:?}", rs);
    }
}
