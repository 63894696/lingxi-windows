//! 引擎冒烟测试:内存 trie 提速 + 模糊音(2026-09-02)。
//! 用真实 ciku.db 验证 (a) build_index 后 query 走内存微秒级 (b) 模糊音 z/zh 生效。
//! 跑: cargo test --release --test smoke -- --nocapture
use prisir_ime::engine::ImeEngine;
use std::time::Instant;

/// 词库路径：优先环境变量 LINGXIME_DB，默认本仓 stage 里的权威词库。
/// （原来写死 voice_input/lingxi_ime/backend/ciku.db —— 那是 11-16 的旧副本，
///   缺 jp 修复与 98 万行抢救，导致一堆“测试失败”其实是词库过期。）
fn db_path() -> String {
    std::env::var("LINGXIME_DB").unwrap_or_else(|_| {
        r"C:\Users\Administrator\oi_enhancements\installer\stage\models\ciku.db".to_string()
    })
}

#[test]
fn sqlite_query_is_fast_enough() {
    // 2026-09-02 决策：mmap 内存索引层被判定会产生残页/跳页/回删卡顿，**已彻底旁路**，
    // 固定走 SQLite（唯一已验证干净的路径）。所以这里不再断言"内存索引更快"，
    // 而是断言"SQLite 路径足够快"——这是当前唯一真实的性能契约。
    let mut e = ImeEngine::new(&db_path()).expect("open db");
    let (ok, src) = e.load_or_build_index(&db_path());
    println!("[smoke] index loaded ok={} src={}（预期 false/sqlite）", ok, src);
    assert!(!ok && src == "sqlite", "内存索引应保持旁路状态");

    // 取 best-of-N 而不是 worst：cargo test 默认并行跑多个测试二进制，
    // mmap_parity 那种几分钟的重活会造成磁盘争用，worst 值量的是机器噪声不是算法成本。
    let mut worst_best = 0.0f64;
    for inp in ["nihao", "shijie", "zhongguo", "duomotai"] {
        let mut best = f64::MAX;
        let mut c = Vec::new();
        for _ in 0..5 {
            let t0 = Instant::now();
            c = e.query(inp);
            best = best.min(t0.elapsed().as_secs_f64() * 1000.0);
        }
        worst_best = worst_best.max(best);
        println!("[smoke] query({:?}) best={:.2}ms, top={:?}", inp, best, c.first());
        assert!(!c.is_empty(), "query({}) 应有候选", inp);
    }
    assert!(worst_best < 30.0, "SQLite 单次查询应 <30ms(best-of-5)，实际最差 {:.2}ms", worst_best);
}

#[test]
fn fuzzy_pinyin_works() {
    let mut e = ImeEngine::new(&db_path()).expect("open db");
    e.set_fuzzy_rules(vec!["z_zh".to_string(), "c_ch".to_string(), "s_sh".to_string()]);
    let _ = e.load_or_build_index(&db_path());
    // 2026-11-17:模糊候选现在**留 10 个槽位**（第 41~50 名），所以 zi（平舌 50+）
    // 也能看到翘舌字了 —— 多翻两页能到，不需要任何开关。这也是最初这条测试的意图。
    for (input, expect) in [
        ("zi", ["至", "之", "直", "质", "制", "致"]),
        ("zen", ["真", "这", "正", "镇", "阵", "针"]),
    ] {
        let cands = e.query(input);
        let words: Vec<&str> = cands.iter().map(|(w, _)| w.as_str()).collect();
        println!("[smoke] fuzzy query('{}') total={} 尾6={:?}", input, words.len(),
                 &words[words.len().saturating_sub(6)..]);
        assert!(!cands.is_empty(), "模糊音查询 {} 应有候选", input);
        let has_zhishe = words.iter().any(|w| expect.contains(w));
        assert!(has_zhishe, "模糊候选应能翻到（{} 的翘舌字），实际 {:?}", input, words);
    }
}

#[test]
fn long_pinyin_query_fast() {
    let mut e = ImeEngine::new(&db_path()).expect("open db");
    let _ = e.load_or_build_index(&db_path());
    // 模拟用户报卡顿的长拼音连续输入
    for p in ["n", "ni", "nih", "niha", "nihao"] {
        let t = Instant::now();
        let c = e.query(p);
        println!("[smoke] query('{}') = {:.3}ms cands={}", p, t.elapsed().as_secs_f64()*1000.0, c.len());
    }
}

#[test]
fn mixed_pinyin_query_fast() {
    // 2026-09-02 初衷：内存索引下混拼应 <3ms。但内存索引层已刻意旁路（残页/跳页根因），
    // SQLite 路径的真实量级是 5~40ms（短输入）/ 35~160ms（反向混拼，见下）。
    // 2026-11-17 优化反向混拼后（length(jp) + substr(key,-3) 索引）：
    //   zo 341→35ms / sjie 341→60ms / nshan 110~160ms（缓存冷热差异大）
    // 本测试只做**回归护栏**：一旦回到旧的 300~660ms 量级就报错。
    let mut e = ImeEngine::new(&db_path()).expect("open db");
    e.set_fuzzy_rules(vec!["z_zh".to_string(), "c_ch".to_string(), "s_sh".to_string()]);
    let _ = e.load_or_build_index(&db_path());
    // 预热（首次会 prepare 语句 + 页调入，不计入稳态）
    for p in ["zo", "nih", "niha", "z", "zhon", "sjie", "nshan"] {
        let _ = e.query(p);
    }
    for (p, budget) in [("zo", 150.0), ("nih", 60.0), ("niha", 60.0), ("z", 60.0),
                        ("zhon", 60.0), ("sjie", 250.0), ("nshan", 250.0)] {
        // best-of-10：并行测试/磁盘争用下 worst 会量到噪声（见上一个测试的注释）
        let mut best = f64::MAX;
        let mut c = Vec::new();
        for _ in 0..10 {
            let t = Instant::now();
            c = e.query(p);
            best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        }
        println!("[smoke] mixed query('{}') best={:.1}ms cands={} (budget {:.0}ms)", p, best, c.len(), budget);
        assert!(best < budget, "混拼/反向混拼 '{}' 应 <{:.0}ms, 实际 best {:.1}ms", p, budget, best);
        assert!(!c.is_empty(), "query('{}') 应有候选", p);
    }
}
