use prisir_ime::engine::ImeEngine;

/// 词库路径：优先环境变量 LINGXIME_DB，默认本仓 stage 里的权威词库。
fn db_path() -> String {
    std::env::var("LINGXIME_DB").unwrap_or_else(|_| {
        r"C:\Users\Administrator\oi_enhancements\installer\stage\models\ciku.db".to_string()
    })
}

/// 九宫格（T9 v2，2026-11-19 重做）回归测试：
///   · 数字串能精确展开出读音（"64" → mi / ni）
///   · 候选是该读音在词库里的干净命中（你/米/泥…），不是前缀噪音
///   · 选定读音后候选收敛到那个读音
#[test]
fn t9_readings_and_candidates() {
    let e = ImeEngine::new(&db_path()).unwrap();

    // "64" → mi / ni 两个读音（"oi" 在词库里没有词，会被过滤掉）
    let r = e.t9_query("64");
    assert!(r.pinyins.iter().any(|p| p == "ni"), "应能展开出 ni，实际 {:?}", r.pinyins);
    assert!(r.pinyins.iter().any(|p| p == "mi"), "应能展开出 mi，实际 {:?}", r.pinyins);
    assert_eq!(r.words.first().map(|c| c.0.as_str()), Some("你"), "首个候选应是你");
    for w in ["你", "米", "泥", "密"] {
        assert!(r.words.iter().any(|c| c.0 == w), "候选里应有 {}，实际 {:?}", w, r.words.iter().take(8).map(|c| &c.0).collect::<Vec<_>>());
    }

    // "64426" → "ni hao"（你好）
    let r2 = e.t9_query("64426");
    assert!(r2.pinyins.iter().any(|p| p == "ni hao"), "应能展开出 ni hao，实际 {:?}", r2.pinyins);
    assert!(r2.words.iter().any(|c| c.0 == "你好"), "候选里应有 你好");

    // 选定读音后收敛
    let sel = e.query_reading_candidates("ni");
    assert_eq!(sel.first().map(|c| c.0.as_str()), Some("你"));
    assert!(sel.iter().any(|c| c.0 == "泥"), "ni 的候选里应有 泥");


    // "3" 只有一个读音 e
    let r3 = e.t9_query("3");
    assert_eq!(r3.pinyins, vec!["e".to_string()], "3 只应展开出 e，实际 {:?}", r3.pinyins);

    // "224326" = caidan：曾因短音节优先 + expand 上限截断，只剩 bai da o / 摆到
    let r4 = e.t9_query("224326");
    assert!(
        r4.pinyins.iter().any(|p| p == "cai dan"),
        "224326 应有 cai dan，实际 {:?}",
        r4.pinyins
    );
    assert_eq!(
        r4.words.first().map(|c| c.0.as_str()),
        Some("菜单"),
        "首候选应为菜单，实际 {:?}",
        r4.words.iter().take(5).map(|c| &c.0).collect::<Vec<_>>()
    );
}

/// 读音展开（**不查库**）：主线程靠它做首行即时反馈，必须是纯内存、微秒级。
#[test]
fn t9_readings_only_expansion() {
    let e = ImeEngine::new(&db_path()).unwrap();

    let rs = e.t9_readings("64");
    assert!(rs.iter().any(|p| p == "ni"), "应展开出 ni，实际 {:?}", rs);
    assert!(rs.iter().any(|p| p == "mi"), "应展开出 mi，实际 {:?}", rs);

    // 拼不出读音时为空（界面据此不画读音条）
    assert!(e.t9_readings("1").is_empty(), "1 不该有读音");
    assert!(e.t9_readings("").is_empty(), "空串不该有读音");

    // 同一词库 key 只留一种读法："ni hao" 与 "ni ha o" 都是 nihao → 留音节少的
    let rs2 = e.t9_readings("64426");
    let nihao = rs2.iter().filter(|p| p.replace(' ', "") == "nihao").count();
    assert_eq!(nihao, 1, "nihao 不该出现两种读法，实际 {:?}", rs2);
}
