//! prisir_ime — Prisir Browser 内置输入法引擎 (IME 线 P0)
//!
//! 逻辑移植自已验证的 Python lingxi_ime(逻辑同源,Python 保留为开发/测试主力)。
//! 词库复用 ciku.db(phrase 186万 + pinyin 33万 + wubi86 13.7万),纯本地零外发。
//!
//! 模块:
//!   syllable  — 标准普通话音节表 + 拼音切分
//!   db        — rusqlite 读 ciku.db(单字/词组/简拼/前缀/反向混拼)
//!   trie      — 全内存 Trie 前缀索引(移植 trie_index.py)
//!   engine    — 候选查询合并排序 + 混拼 + 模糊音 + DP 整句智能首选
//!   lang      — P0 中英文自动判定(音节可切分性 + 词库命中两级判定)
//!   punct     — P0 智能标点(ASCII→中文标点 + 引号配对状态机)
//!   lattice   — P1 整句解码(字节跨度网格 + 束搜索,全拼/简拼/混拼/单字母统一)
//!   s2t       — P2 简繁转换(OpenCC 数据内嵌,最长短语匹配 + 单字回退)

pub mod syllable;
pub mod db;
pub mod trie;
pub mod mmap_index;
pub mod engine;
pub mod lang;
pub mod punct;
pub mod lattice;
pub mod s2t;
pub mod ffi;

#[cfg(target_os = "android")]
pub mod jni;

// 手写 / 搜狗 gpen 对照已下线（2026-11-19）：
//   · 自研 ONNX 手写认不准（"识"一直认不出）
//   · 搜狗 20.20 的手写是云端/内置资源，拿不到可离线加载的模型（见 Obsidian 逆向报告）
// 逆向成果保留在 prisir_ime/src/gpen.rs 与 ~/oi_enhancements/gpen_probe（都不入产品包）。

pub use engine::ImeEngine;
