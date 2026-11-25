//! about.rs — 灵犀拼音输入法 Win 端说明文案
//!
//! 文案结构对齐 Linux `helpers/prisir_about`：
//!   关于 · 隐私政策 · 使用条款 · 开放 API · 问题反馈
//! 另保留「使用说明」（Android AboutCopy 亦有；Win 安装启用步骤不同）。
//! 开源协议：AGPL-3.0（与安装包 LICENSE.txt 一致）。反馈仅走 BabelSpan 论坛。

/// 官网与论坛（与 Linux / Android 一致）。
pub const WEBSITE_URL: &str = "https://www.babelspan.com/";
pub const FORUM_URL: &str =
    "https://bbs.babelspan.com/forum.html#board=browser/ime&hint=prisirai";

/// 使用说明（Windows / TSF；Linux 端无此栏，因 fcitx5 启用路径不同）。
pub const HELP_BODY: &str = "\
启用输入法：\n\
系统设置 → 时间和语言 → 语言和区域 → 中文 → 语言选项 → 添加键盘 → 选择「灵犀拼音」，并切换为当前输入法。\n\
\n\
基本操作：\n\
· 输入拼音后，点选候选或按空格上屏；数字键可选对应候选。\n\
· 悬浮状态栏可切换中/英、中英标点、简繁输出，并打开手写、符号、表情、词库与语音听写。\n\
· 「造词」可把当前选中文本或剪贴板内容加入个人词库（也可在状态栏菜单进入；热键可在配置中调整）。\n\
· 词库管理支持搜索、删除与导入导出；学习记录保存在本机用户目录。\n\
· 开放 API 默认关闭；需要本机 AI Agent 协助定制时，请参阅「开放 API」说明后开启。\n\
\n\
问题与诊断包请通过「问题反馈」或「反馈问题(打包诊断)」提交到论坛。";

/// 关于（对齐 Linux ABOUT_BODY；平台句改为 Windows / TSF；协议 AGPL-3.0）。
pub const ABOUT_BODY: &str = "\
灵犀拼音输入法（Windows / TSF）由 BabelSpan · Prisir（湃睿思）提供。\n\
\n\
产品定位：面向个人与专业场景的本地中文输入工具。核心能力包括拼音/混拼候选、整句解码、简繁输出、本地词库与用户词典管理。\n\
\n\
手写走系统 Ink（精度高于自研 ONNX/安卓同款方案）。\n\
语音听写为可选组件（SenseVoice Small / onnxruntime），在本机完成识别。\n\
不做 AI 助手菜单（属独立 PrisirAI 产品）。Windows 端不内置翻译面板。\n\
\n\
若需借助 AI Agent 定制输入法，请用菜单「开放 API」：默认关闭，仅 127.0.0.1。\n\
\n\
官网：https://www.babelspan.com/\n\
\n\
版权所有 © 2026 BabelSpan。本软件依据 GNU Affero General Public License v3.0（AGPL-3.0）授权，按「原样」提供。完整授权见安装目录 LICENSE.txt（AGPL-3.0）。";

/// 隐私政策（对齐 Linux PRIVACY_BODY）。
pub const PRIVACY_BODY: &str = "\
灵犀拼音采用严格的隐私保护设计，完全离线无账号运行：\n\
\n\
· 键入内容、候选结果与词库数据保存在本机，不向第三方上传输入正文。\n\
· 不提供账号登录，不做行为画像，不出售或交换个人输入数据。\n\
· 用户可通过词库管理自主维护本地词典。\n\
· 可选的语音听写在本机完成识别，不外发音频。\n\
· 开放 API 默认关闭；开启后也仅监听本机 127.0.0.1。\n\
\n\
输入法本身不主动发起任何网络请求。";

/// 使用条款（对齐 Linux TERMS_BODY；协议 AGPL-3.0）。
pub const TERMS_BODY: &str = "\
使用本软件即表示您知悉并同意：\n\
\n\
· 本软件为本地工具，安装、配置与使用风险由用户自行承担。\n\
· 请仅将本软件用于合法用途。\n\
· 词库与学习数据归用户本地管理；卸载前请自行备份。\n\
· 完整法律文本以安装目录 LICENSE.txt（GNU Affero General Public License v3.0）为准。\n\
\n\
官网：https://www.babelspan.com/";

/// 开放 API（说明文案；实际开关/端口见设置窗 openapi_window）。
pub const OPENAPI_BODY: &str = "\
开放 API 面向「用 AI 帮助用户定制个性化输入法」：本机 Agent / 脚本可通过本地接口查询候选、学习词条、简繁与状态。\n\
\n\
默认关闭。请在状态栏菜单打开「开放 API」设置页：可开关服务并自定义端口（默认 23331），仅监听 127.0.0.1。\n\
\n\
配置：%LOCALAPPDATA%\\Prisir\\IME\\open_api.json\n\
端点：GET /health · GET /status · POST /rpc\n\
\n\
契约对齐 Linux / Android。不需要 Agent 时可保持关闭。";

/// 问题反馈（合并原「问题反馈」与「反馈问题(打包诊断)」）。
pub const FEEDBACK_BODY: &str = "\
如需报告缺陷或提出改进建议，请通过 BabelSpan 论坛提交：\n\
\n\
https://bbs.babelspan.com/forum.html#board=browser/ime&hint=prisirai\n\
\n\
建议说明：Windows 版本与灵犀版本号、复现步骤、期望与实际结果。\n\
\n\
下方「打包诊断」会在桌面生成诊断包（日志与脱敏环境信息，不含输入正文），并打开论坛便于上传。";

/// 兼容旧 key。
pub const COPYRIGHT_BODY: &str = "\
版权所有 © 2026 BabelSpan。本软件依据 GNU Affero General Public License v3.0（AGPL-3.0）授权，按「原样」提供。完整授权见安装目录 LICENSE.txt。官网：https://www.babelspan.com/";

/// 段落标题（与 Linux SECTIONS + 使用说明 对齐）。
pub const ABOUT_TITLES: &[&str] = &[
    "关于",
    "使用说明",
    "隐私政策",
    "使用条款",
    "开放 API",
    "问题反馈",
];

/// 查表：返回 (title, body)。
pub fn lookup(key: &str) -> Option<(&'static str, &'static str)> {
    match key {
        "help" | "usage" => Some(("使用说明", HELP_BODY)),
        "about" => Some(("关于", ABOUT_BODY)),
        "copyright" => Some(("版权声明", COPYRIGHT_BODY)),
        "privacy" => Some(("隐私政策", PRIVACY_BODY)),
        "terms" => Some(("使用条款", TERMS_BODY)),
        "openapi" | "api" => Some(("开放 API", OPENAPI_BODY)),
        "contact" | "feedback" => Some(("问题反馈", FEEDBACK_BODY)),
        _ => None,
    }
}

/// `--version` 末尾「更多」提示。
pub fn about_links_inline() -> String {
    "更多: 关于/使用说明/隐私政策/使用条款/开放API/问题反馈 — 跑 `prisir_tsfsvc --about <about|help|privacy|terms|openapi|feedback>` 查看正文; 论坛见菜单「问题反馈」".into()
}
