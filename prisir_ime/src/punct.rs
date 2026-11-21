//! P0 智能标点（2026-11-16）
//!
//! 对标搜狗 `libpunctuator.so` / 豆包内置标点：中文模式下把 ASCII 标点映射成中文标点，
//! 并对成对符号（引号）做开/合状态机。
//!
//! 设计边界：
//!   - **只在中/英文判定为"中文"时启用**；自动判英文上屏时保持 ASCII 原样。
//!   - **只在拼音缓冲为空时启用**；缓冲非空时 `.` `,` 仍是翻页键（沿用既有行为）。
//!   - 状态机是**进程级**的（引擎句柄单例）。多输入框切换靠 `reset` 归位。

/// 引号配对状态。
#[derive(Clone, Copy, Default, Debug)]
pub struct PunctState {
    /// 双引号当前处于"已开"状态（下一次该出右引号）
    pub dquote_open: bool,
    /// 单引号当前处于"已开"状态
    pub squote_open: bool,
}

impl PunctState {
    pub fn reset(&mut self) {
        self.dquote_open = false;
        self.squote_open = false;
    }
}

/// ASCII 标点 → 中文标点。返回 `None` 表示不做映射（交回原按键处理）。
///
/// 映射取自主流中文输入法（搜狗/微软/豆包）的默认键位习惯。
pub fn map(ch: char, st: &mut PunctState) -> Option<&'static str> {
    let out = match ch {
        ',' => "，",
        '.' => "。",
        '?' => "？",
        '!' => "！",
        ';' => "；",
        ':' => "：",
        '(' => "（",
        ')' => "）",
        '[' => "【",
        ']' => "】",
        '{' => "『",
        '}' => "』",
        '<' => "《",
        '>' => "》",
        '\\' => "、",
        '$' => "￥",
        '~' => "～",
        '_' => "——",
        '^' => "……",
        // 引号：开/合交替
        '"' => {
            if st.dquote_open {
                st.dquote_open = false;
                "”"
            } else {
                st.dquote_open = true;
                "“"
            }
        }
        '\'' => {
            if st.squote_open {
                st.squote_open = false;
                "’"
            } else {
                st.squote_open = true;
                "‘"
            }
        }
        _ => return None,
    };
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_punct() {
        let mut st = PunctState::default();
        assert_eq!(map(',', &mut st), Some("，"));
        assert_eq!(map('.', &mut st), Some("。"));
        assert_eq!(map('?', &mut st), Some("？"));
        assert_eq!(map('!', &mut st), Some("！"));
        assert_eq!(map('\\', &mut st), Some("、"));
        assert_eq!(map('<', &mut st), Some("《"));
        assert_eq!(map('>', &mut st), Some("》"));
    }

    #[test]
    fn quotes_pair_and_toggle() {
        let mut st = PunctState::default();
        assert_eq!(map('"', &mut st), Some("“"));
        assert_eq!(map('"', &mut st), Some("”"));
        assert_eq!(map('"', &mut st), Some("“"));
        assert_eq!(map('\'', &mut st), Some("‘"));
        assert_eq!(map('\'', &mut st), Some("’"));
    }

    #[test]
    fn reset_closes_quotes() {
        let mut st = PunctState::default();
        map('"', &mut st);
        assert!(st.dquote_open);
        st.reset();
        assert_eq!(map('"', &mut st), Some("“"));
    }

    #[test]
    fn unmapped_passthrough() {
        let mut st = PunctState::default();
        // 字母、空格、以及中文输入法通常保留原样的符号
        assert_eq!(map('a', &mut st), None);
        assert_eq!(map(' ', &mut st), None);
        assert_eq!(map('@', &mut st), None);
        assert_eq!(map('+', &mut st), None);
        assert_eq!(map('=', &mut st), None);
        assert_eq!(map('/', &mut st), None);
        assert_eq!(map('#', &mut st), None);
        assert_eq!(map('&', &mut st), None);
        assert_eq!(map('*', &mut st), None);
    }

    #[test]
    fn ellipsis_and_dash() {
        let mut st = PunctState::default();
        assert_eq!(map('^', &mut st), Some("……"));
        assert_eq!(map('_', &mut st), Some("——"));
    }
}
