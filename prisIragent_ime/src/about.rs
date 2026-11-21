//! about.rs — 灵犀拼音输入法 Windows 端说明文案
//!
//! 菜单分栏：关于 · 隐私政策 · 使用条款 · 开放 API · 问题反馈
//! 亦供 `prisir_tsfsvc --about <key>` 与 `--version` 尾注使用。

/// 产品官网（介绍页与下载）。
pub const WEBSITE_URL: &str = "https://www.babelspan.com/";

/// 用户论坛（问题反馈入口）。
pub const FORUM_URL: &str = "https://bbs.babelspan.com/forum.html#board=browser/ime&hint=prisirai";

/// 关于
pub const ABOUT_BODY: &str = "\
灵犀拼音输入法（Windows）由 BabelSpan · Prisir（湃睿思）提供。\n\
\n\
产品定位：面向个人与专业场景的本地中文输入工具。核心能力包括拼音/混拼候选、整句解码、简繁输出、本地词库与用户词典管理，以及可选的语音听写插件。输入链路在本机完成，不依赖账号体系，也不以云端联想为默认能力。\n\
\n\
若需借助 AI Agent 按个人习惯定制输入法（词库、模式、查询等），请参阅菜单「开放 API」：默认关闭，按需开启后仅对本机回环地址开放。\n\
\n\
官网与下载：https://www.babelspan.com/\n\
\n\
版权所有 © 2026 BabelSpan。本软件依据 Apache License, Version 2.0 授权，按「原样」提供，不附带任何担保或条件。完整授权见安装目录 LICENSE.txt（Apache License 2.0）。";

/// 隐私政策
pub const PRIVACY_BODY: &str = "\
灵犀拼音采用严格的隐私保护设计，完全离线无账号运行：\n\
\n\
· 键入内容、候选结果与词库数据保存在本机，不向第三方上传输入正文。\n\
· 不提供账号登录，不做行为画像，不出售或交换个人输入数据。\n\
· 用户可通过词库管理、黑白名单与学习记录自主维护本地词典。\n\
· 可选的语音听写插件在本机完成识别，不外发音频。\n\
· 开放 API 默认关闭；开启后也仅监听本机 127.0.0.1，不对外网暴露。\n\
\n\
输入法本身不主动发起任何网络请求。";

/// 使用条款
pub const TERMS_BODY: &str = "\
使用本软件即表示您知悉并同意以下约定：\n\
\n\
· 本软件为本地工具，安装、配置与使用风险由用户自行承担。\n\
· 请仅将本软件用于合法用途；请勿用于干扰系统安全或侵犯他人权益的场景。\n\
· 词库与学习数据归用户本地管理；卸载前请自行备份需要保留的用户词典。\n\
· 软件可能通过官网发布更新；是否安装更新由用户决定。\n\
\n\
完整法律文本以安装目录 LICENSE.txt（Apache License, Version 2.0）为准。官网：https://www.babelspan.com/";

/// 开放 API 说明
pub const OPENAPI_BODY: &str = "\
开放 API 面向「用 AI 帮助用户定制个性化输入法」的场景：本机 Agent / 脚本可通过本地接口查询状态、调整配置、访问词库相关能力，从而按个人习惯协助设置输入法。\n\
\n\
默认关闭。需要 Agent 接入时请先开启：\n\
\n\
  prisir_tsfsvc --open-api-config enable\n\
\n\
开启后仅监听本机 127.0.0.1（默认端口 23331），不对外网开放。配置文件：%LOCALAPPDATA%\\Prisir\\IME\\open_api.json。\n\
\n\
常用端点：GET /health · GET /status · POST /rpc（JSON-RPC）。\n\
关闭：prisir_tsfsvc --open-api-config disable\n\
查看：prisir_tsfsvc --open-api-config show\n\
\n\
请仅在信任本机环境时开启；不需要个性化 Agent 协助时可保持关闭。";

/// 问题反馈说明
pub const FEEDBACK_BODY: &str = "\
如需报告缺陷或提出改进建议，请通过 BabelSpan 论坛提交：\n\
\n\
https://bbs.babelspan.com/forum.html#board=browser/ime&hint=prisirai\n\
\n\
建议在帖中说明：Windows 版本与灵犀版本号、复现步骤、期望与实际结果。\n\
\n\
菜单「问题反馈」将自动在桌面生成诊断包（日志与脱敏环境信息，不含输入正文），并打开论坛页面，便于您上传附件。";

/// 兼容旧 key 名；版权已并入 ABOUT_BODY。
pub const COPYRIGHT_BODY: &str = ABOUT_BODY;

/// 菜单与 CLI 标题顺序。
pub const ABOUT_TITLES: &[&str] = &["关于", "隐私政策", "使用条款", "开放 API", "问题反馈"];

/// `--about <key>` → (title, body)
pub fn lookup(key: &str) -> Option<(&'static str, &'static str)> {
    match key {
        "about" | "copyright" => Some(("关于", ABOUT_BODY)),
        "privacy" => Some(("隐私政策", PRIVACY_BODY)),
        "terms" => Some(("使用条款", TERMS_BODY)),
        "openapi" | "open_api" | "api" => Some(("开放 API", OPENAPI_BODY)),
        "feedback" | "contact" => Some(("问题反馈", FEEDBACK_BODY)),
        _ => None,
    }
}

/// `--version` 尾注。
pub fn about_links_inline() -> String {
    format!(
        "更多：关于 / 隐私政策 / 使用条款 / 开放 API / 问题反馈 — `prisir_tsfsvc --about <about|privacy|terms|openapi|feedback>`；官网 {} ；论坛 {}",
        WEBSITE_URL, FORUM_URL
    )
}
