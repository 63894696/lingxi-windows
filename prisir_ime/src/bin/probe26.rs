//! 26 键简拼 / 九宫格学成词 的快速探针（**只读**，不重建 .midx）。
//!
//! 用法：`cargo run --release --bin probe26 -- <db_path>`
//!
//! 为什么单独一个 bin：`midxtool` 会**重建** `.midx`（几十秒 + 改文件），
//! 这里只想快速回答两个问题：
//!   ① 26 键打简拼能不能一次出多字词？解析出的读音对不对？
//!      （用户反馈"九宫格能一次出四个字，26 键能不能"）
//!   ② 九宫格候选有没有合并**用户学过的词**（handoff 下一步 #3）
//!
//! ② 的验证会临时写一条 user_word 并立刻删掉（不污染学习库）。

use prisir_ime::engine::ImeEngine;

fn main() {
    let db = std::env::args().nth(1).expect("usage: probe26 <db_path>");
    let mut e = ImeEngine::new(&db).expect("open db");
    let (ok, src) = e.load_mmap_index(&db);
    println!("[probe26] db={} mmap_ok={} src={}", db, ok, src);

    // ① 26 键简拼：解析读音 + 候选
    for probe in [
        "nhjsg", "nhxf", "nhm", "nihao", "nh", "nha", "nhao", "zg", "shjs", "womens",
    ] {
        let t = std::time::Instant::now();
        let r = e.resolve_pinyin(probe);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let c = e.query(probe);
        let top = c.iter().take(4).map(|(w, _)| w.clone()).collect::<Vec<_>>();
        // 界面真正用的是「按首候选词反查」——必须与首候选一致
        let by_word = top.first().and_then(|w| e.pinyin_of_word(w));
        println!(
            "[probe26] {:>7} resolve={:?} ({:.2}ms) top4={:?}\n          按首候选词反查={:?}",
            probe,
            r,
            ms,
            top,
            by_word
        );
    }

    // ② 九宫格：学成词必须置顶（写完立刻删）
    let _ = e.db.add_user_word("nihao", "泥嚎探针", 1);
    for d in ["646", "64", "64426"] {
        let r = e.t9_query(d);
        println!(
            "[probe26] t9_query('{}') pinyins={:?} top6={:?}",
            d,
            r.pinyins,
            r.words.iter().take(6).map(|(w, _)| w.clone()).collect::<Vec<_>>()
        );
    }
    let n = e.db.remove_user_word("nihao", "泥嚎探针").unwrap_or(0);
    println!("[probe26] cleanup removed={} (探针词已删除)", n);
}
