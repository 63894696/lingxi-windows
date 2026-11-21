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
fn dbg_zo() {
    let mut e = ImeEngine::new(&db_path()).unwrap();
    e.set_fuzzy_rules(vec!["z_zh".to_string(),"c_ch".to_string(),"s_sh".to_string()]);
    let (ok, src) = e.load_or_build_index(&db_path());
    println!("[zo] index ok={} src={} mem={}", ok, src, e.mem.is_some());
    // 全查询计时
    for _ in 0..3 {
        let t = Instant::now();
        let c = e.query("zo");
        println!("[zo] full query = {:.3}ms cands={}", t.elapsed().as_secs_f64()*1000.0, c.len());
    }
    // 拆: is_full_pinyin / segment
    let t = Instant::now(); let segs = e.segment("zo"); println!("[zo] segment(zo)={:?} {:.3}ms", segs, t.elapsed().as_secs_f64()*1000.0);
    // 拆: trie 各通道
    if let Some(mem) = &e.mem {
        let t = Instant::now(); let s = mem.query_prefix_single("zo"); println!("[zo] mem single = {:.3}ms n={}", t.elapsed().as_secs_f64()*1000.0, s.len());
        let t = Instant::now(); let p = mem.query_prefix("zo"); println!("[zo] mem prefix = {:.3}ms n={}", t.elapsed().as_secs_f64()*1000.0, p.len());
        let t = Instant::now(); let jp = mem.query_jp("zo", 20); println!("[zo] mem jp = {:.3}ms n={}", t.elapsed().as_secs_f64()*1000.0, jp.len());
        let t = Instant::now(); let rm = mem.query_reverse_mixed("z", "o", 15); println!("[zo] mem rev(z,*o) = {:.3}ms n={}", t.elapsed().as_secs_f64()*1000.0, rm.len());
    }
}
