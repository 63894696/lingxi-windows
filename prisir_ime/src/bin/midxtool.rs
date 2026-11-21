//! 生成 `<db>.midx`（mmap 内存索引）文件 —— Android 端用。
//!
//! 用法：`midxtool <db_path>`
//!
//! 为什么需要它：手机上的词库查询必须 µs 级，否则打字会"卡手"。
//! SQLite 每次查询要走 SQL 解析 + 页缓存；mmap 索引是**映射即用、零解析**，
//! 且物理页由内核管理（可回收、多进程共享），比在手机上重建一份 HashMap 树省内存得多。
//!
//! 注意：`.midx` 里记了词库指纹（长度 + mtime），词库一换索引就作废，
//! 下发流程必须"词库和 .midx 一起更新"。

use prisir_ime::engine::ImeEngine;
use std::time::Instant;

fn main() {
    let db = std::env::args().nth(1).expect("usage: midxtool <db_path>");
    println!("[midxtool] db={}", db);
    let md = std::fs::metadata(&db).expect("stat db");
    println!("[midxtool] db_len={}", md.len());

    let mut e = ImeEngine::new(&db).expect("open db");
    e.set_fuzzy_rules(vec!["z_zh".into(), "c_ch".into(), "s_sh".into()]);

    // 1) 全量重建内存索引并落盘成 .midx
    let t = Instant::now();
    let n = e
        .build_and_save_mmap_index(&db)
        .expect("build and save midx");
    let midx = std::path::Path::new(&db).with_extension("midx");
    println!(
        "[midxtool] wrote {} ({} bytes) took={:.1}ms",
        midx.display(),
        n,
        t.elapsed().as_secs_f64() * 1000.0
    );

    // 2) 自检：重新打开、加载 .midx、查几次看延迟
    let mut e2 = ImeEngine::new(&db).expect("open db2");
    let t = Instant::now();
    let (ok, src) = e2.load_mmap_index(&db);
    println!(
        "[midxtool] load_mmap_index -> ok={} src={} took={:.2}ms",
        ok,
        src,
        t.elapsed().as_secs_f64() * 1000.0
    );
    if !ok {
        return;
    }
    // 3) SQLite 路径(e.mem=None) vs mmap 路径(e2.mem=Some):候选一致性 + 延迟对比。
    //    一致性是硬门槛:三端(Windows/Linux/Android)必须给出同一批候选。
    println!("[midxtool] --- SQLite vs mmap 对比(候选一致性 + 延迟)---");
    let mut mismatch = 0usize;
    for probe in ["ni", "nihao", "shi", "zhongguo", "xian", "s", "a"] {
        // SQLite 路径:跑 3 次取最快(避开首次页缓存冷启动)
        let mut sqlite_ms = f64::MAX;
        let mut sq = Vec::new();
        for _ in 0..3 {
            let t = Instant::now();
            sq = e.query(probe);
            sqlite_ms = sqlite_ms.min(t.elapsed().as_secs_f64() * 1000.0);
        }
        // mmap 路径
        let mut mmap_ms = f64::MAX;
        let mut mq = Vec::new();
        for _ in 0..3 {
            let t = Instant::now();
            mq = e2.query(probe);
            mmap_ms = mmap_ms.min(t.elapsed().as_secs_f64() * 1000.0);
        }
        let a: Vec<&String> = sq.iter().take(10).map(|(w, _)| w).collect();
        let b: Vec<&String> = mq.iter().take(10).map(|(w, _)| w).collect();
        let same = a == b;
        if !same {
            mismatch += 1;
        }
        println!(
            "[midxtool] query('{}'): sqlite={:.2}ms/{}cands  mmap={:.2}ms/{}cands  head10={}",
            probe,
            sqlite_ms,
            sq.len(),
            mmap_ms,
            mq.len(),
            if same { "✓一致" } else { "✗不一致" }
        );
        if !same {
            println!("    sqlite={:?}\n    mmap  ={:?}", a, b);
        }
    }
    assert_eq!(mismatch, 0, "mmap 路径与 SQLite 路径候选不一致(关键路径不许假 PASS)");
    for probe in ["ni", "nihao", "shi", "zhongguo", "xian", "nh", "nhjsg", "zg"] {
        let mut times = Vec::new();
        let mut top = Vec::new();
        let mut n = 0;
        for _ in 0..5 {
            let t = Instant::now();
            let c = e2.query(probe);
            times.push(t.elapsed().as_secs_f64() * 1000.0);
            n = c.len();
            if top.is_empty() {
                top = c.iter().take(3).map(|(w, _)| w.clone()).collect();
            }
        }
        println!(
            "[midxtool] mmap query('{}') -> {} cands, times={:?}ms top3={:?}",
            probe,
            n,
            times.iter().map(|x| (x * 1000.0).round() / 1000.0).collect::<Vec<_>>(),
            top
        );
    }
    let t = Instant::now();
    let r = e2.t9_query("64");
    println!(
        "[midxtool] mmap t9_query('64') -> {} cands in {:.3}ms",
        r.words.len(),
        t.elapsed().as_secs_f64() * 1000.0
    );
    // 九宫格读音展开：中间态（拼不出整串）是否有读音 —— 断了就只剩"整串命中"的杂候选
    for d in ["6", "64", "646", "6442", "9464", "2", "22"] {
        let rs = e2.t9_readings(d);
        println!("[midxtool] t9_readings('{}') -> {} 条 {:?}", d, rs.len(), rs);
    }
    // 26 键简拼/混拼
    for probe in ["nhao", "nha", "nhjsg"] {
        let c = e2.query(probe);
        println!(
            "[midxtool] mmap query('{}') -> {} cands top5={:?}",
            probe,
            c.len(),
            c.iter().take(5).map(|(w, _)| w.clone()).collect::<Vec<_>>()
        );
    }
}
