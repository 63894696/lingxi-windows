use prisir_ime::engine::ImeEngine;
/// 词库路径：优先环境变量 LINGXIME_DB，默认本仓 stage 里的权威词库。
/// （原来写死 voice_input/lingxi_ime/backend/ciku.db —— 那是 11-16 的旧副本，
///   缺 jp 修复与 98 万行抢救，导致一堆“测试失败”其实是词库过期。）
fn db_path() -> String {
    std::env::var("LINGXIME_DB").unwrap_or_else(|_| {
        r"C:\Users\Administrator\oi_enhancements\installer\stage\models\ciku.db".to_string()
    })
}
#[test]
fn dbg_zi() {
    let mut e = ImeEngine::new(&db_path()).unwrap();
    e.set_fuzzy_rules(vec!["z_zh".to_string(),"c_ch".to_string(),"s_sh".to_string()]);
    let _ = e.load_or_build_index(&db_path());
    let c = e.query("zi");
    println!("[zi] total cands = {}", c.len());
    // 查重复
    let mut seen = std::collections::HashSet::new();
    let mut dup = Vec::new();
    for (w,_) in &c {
        if !seen.insert(w.clone()) { dup.push(w.clone()); }
    }
    println!("[zi] duplicates = {:?}", dup);
    // 找 恣赀吱耔辎 在候选里的位置
    for target in ["恣","赀","吱","耔","辎"] {
        let pos: Vec<usize> = c.iter().enumerate().filter(|(_,(w,_))| w==target).map(|(i,_)| i).collect();
        println!("[zi] '{}' at idx {:?}", target, pos);
    }
    // 全量打印(带页号, 每页5)
    for (i,(w,wt)) in c.iter().enumerate() {
        println!("[zi] p{}#{} idx{} {} w={}", i/5, i%5, i, w, wt);
    }
}
