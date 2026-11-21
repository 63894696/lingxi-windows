use rusqlite::Connection;
/// 词库路径：优先环境变量 LINGXIME_DB，默认本仓 stage 里的权威词库。
/// （原来写死 voice_input/lingxi_ime/backend/ciku.db —— 那是 11-16 的旧副本，
///   缺 jp 修复与 98 万行抢救，导致一堆“测试失败”其实是词库过期。）
fn db_path() -> String {
    std::env::var("LINGXIME_DB").unwrap_or_else(|_| {
        r"C:\Users\Administrator\oi_enhancements\installer\stage\models\ciku.db".to_string()
    })
}
#[test]
fn dbg_bucket() {
    let conn = Connection::open(&db_path()).unwrap();
    // z 开头的 jp 首字母词组数(= 反混 z 桶大小)
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM phrase WHERE jp >= 'z' AND jp < '{'", [], |r| r.get(0)).unwrap();
    println!("[bucket] jp 首字母=z 的 phrase 行数 = {}", n);
    // key 以 o 结尾的 z 开头词组数(= zo 反混命中数)
    let m: i64 = conn.query_row("SELECT COUNT(*) FROM phrase WHERE jp >= 'z' AND jp < '{' AND key LIKE '%o'", [], |r| r.get(0)).unwrap();
    println!("[bucket] 其中 key 以 'o' 结尾 = {}", m);
    // 全 phrase 行数
    let t: i64 = conn.query_row("SELECT COUNT(*) FROM phrase", [], |r| r.get(0)).unwrap();
    println!("[bucket] phrase 总行数 = {}", t);
}
