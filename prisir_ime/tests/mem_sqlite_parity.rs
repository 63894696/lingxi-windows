//! 内存(mmap)路径 vs SQLite 路径的候选一致性(关键路径不许假 PASS)。
//!
//! 背景(2026-11-19):Android 端要走 mmap 内存索引,但发现 ——
//!   `MemoryIndex` 的 top/top1 桶是**前缀累积**语义(insert 沿路径每层都插),
//!   而 SQLite 的 `WHERE key=?` 是**精确**语义。
//!   → 内存路径查 "ni" 会把 "niao"/"nie" 的词也带出来,与 SQLite 结果不同,
//!     三端(Windows/Linux/Android)候选不一致 —— 这是硬伤,必须锁死。
//!
//! 修法:给索引加**精确桶**(own / own1,只在终点节点写入),
//!   内存路径改用 query_own / query_own_single,与 SQLite 的
//!   `query_phrase(key)` / `query_pinyin(key)` 逐条对齐。
//!
//! 断言口径:**前 10 个候选完全一致**(词 + 权重 + 顺序)。
//!   不要求全等:精确桶 cap=32 而 SQLite 通道 LIMIT=20/50,
//!   union 后尾部条数允许有量差(不影响用户看到的首屏候选)。
//!
//! 运行: LINGXIME_DB=<词库> cargo test --release --test mem_sqlite_parity -- --nocapture

use prisir_ime::engine::ImeEngine;

/// 词库路径:优先环境变量 LINGXIME_DB,默认本仓 stage 里的权威词库。
fn db_path() -> String {
    std::env::var("LINGXIME_DB").unwrap_or_else(|_| {
        r"C:\Users\Administrator\oi_enhancements\installer\stage\models\ciku.db".to_string()
    })
}

/// 代表性输入:覆盖单音节/多音节/单字母/混拼/不存在。
const INPUTS: &[&str] = &[
    "ni", "nih", "nihao", "shi", "zhong", "wo", "zi", "zhi", "s", "xian", "zhongguo", "womenshi",
    "j", "a", "e", "qwe", "zxcv",
];

/// 九宫格读音(走 query_reading_candidates,与九宫格候选窗一致)。
const READINGS: &[&str] = &["ni", "shi", "nihao", "zhongguo", "wo", "shi", "ai"];

fn head10(v: &[(String, i64)]) -> Vec<(String, i64)> {
    v.iter().take(10).cloned().collect()
}

#[test]
fn mem_matches_sqlite() {
    let path = db_path();
    let mut e = ImeEngine::new(&path).expect("open db");

    // 1) SQLite 路径(mem=None)—— 基准
    let mut base: Vec<(&str, Vec<(String, i64)>)> = Vec::new();
    for inp in INPUTS {
        base.push((inp, e.query(inp)));
    }
    let mut base_r: Vec<(&str, Vec<(String, i64)>)> = Vec::new();
    for r in READINGS {
        base_r.push((r, e.query_reading_candidates(r)));
    }

    // 2) 建内存索引 → mem 路径
    e.build_memory_index().expect("build mem index");

    let mut fail = 0;
    for (inp, sq) in &base {
        let mq = e.query(inp);
        let (a, b) = (head10(sq), head10(&mq));
        if a != b {
            println!("MISMATCH[query] input={inp:?}\n  sqlite={a:?}\n  mem   ={b:?}");
            fail += 1;
        }
    }
    for (r, sq) in &base_r {
        let mq = e.query_reading_candidates(r);
        let (a, b) = (head10(sq), head10(&mq));
        if a != b {
            println!("MISMATCH[reading] input={r:?}\n  sqlite={a:?}\n  mem   ={b:?}");
            fail += 1;
        }
    }

    println!("[mem_sqlite_parity] {} inputs + {} readings checked, {} mismatches",
        INPUTS.len(), READINGS.len(), fail);
    assert_eq!(fail, 0, "内存(mmap)路径与 SQLite 路径候选不一致(关键路径不许假 PASS)");
}
