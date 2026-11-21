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
fn dbg_load() {
    // 模拟首次切换: ImeEngine::new + load_or_build_index(走 cache) 全程计时
    let t = Instant::now();
    let mut e = ImeEngine::new(&db_path()).unwrap();
    println!("[load] ImeEngine::new (open db) = {:.3}ms", t.elapsed().as_secs_f64()*1000.0);
    let t = Instant::now();
    let (ok, src) = e.load_or_build_index(&db_path());
    println!("[load] load_or_build_index src={} ok={} = {:.3}ms", src, ok, t.elapsed().as_secs_f64()*1000.0);
}
