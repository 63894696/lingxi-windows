//! 混拼 / 中英判定 / 整句首选 探针(只读,不重建 .midx)。
//!
//! 用法:`cargo run --release --bin mixdiag -- <db_path> <输入1> [输入2 ...]`
//!
//! 打印每条输入的四件事,对应 2026-11-25 用户报的三个 bug:
//!   1. `query()` 首屏候选 —— "候选窗第一行显示的是不是那个词";
//!   2. `classify_input()` —— 判成英文会把候选换成原样字母(混拼打不出来的真因);
//!   3. `smart_sentence()` —— 空格上屏是否会跑整句;
//!   4. **模拟空格上屏**:引擎已经把整句补进候选第 1 位,所以 `query()[0]` 就是
//!      按空格会出什么 —— 用来验证「显示 == 上屏」。
//!
//! 例:
//!   mixdiag ciku.db sidegeer limancx yakebicx jintiantianqi
use prisir_ime::engine::ImeEngine;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let db = args.next().expect("usage: mixdiag <db_path> <input...>");
    let probes: Vec<String> = args.collect();
    assert!(!probes.is_empty(), "至少给一个输入串");

    let mut e = ImeEngine::new(&db).expect("open db");
    e.set_fuzzy_rules(vec!["z_zh".into(), "c_ch".into(), "s_sh".into()]);
    let (ok, src) = e.load_mmap_index(&db);
    println!("[mixdiag] db={} mmap_ok={} src={}", db, ok, src);

    for p in &probes {
        let t = Instant::now();
        let c = e.query(p);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let kind = e.classify_input(p);
        let sent = e.smart_sentence(p);
        let top8: Vec<String> = c.iter().take(8).map(|(w, _)| w.clone()).collect();
        println!(
            "\n[mixdiag] 输入 {:>16}  ({:.2}ms, {} 条)\n  classify = {:?}\n  smart_sentence = {:?}\n  空格上屏(=候选第1位) = {:?}\n  top8 = {:?}",
            p,
            ms,
            c.len(),
            kind,
            sent,
            c.first().map(|(w, _)| w.clone()),
            top8
        );
    }
}
