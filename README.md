# 灵犀拼音 · Windows

本地中文拼音输入法（Windows TSF）。与 [Android](https://github.com/63894696/lingxi-android)、[Linux](https://github.com/63894696/lingxi-linux) 共用同一套 Rust 引擎，候选、整句与词库学习行为一致。

**官网下载**：https://www.babelspan.com/prisir-ime.html  
**许可证**：GNU AGPL-3.0-or-later（见 [LICENSE](LICENSE)）

## 功能概览

- **拼音输入**：全拼 / 混拼候选、整句解码、学习上屏
- **简繁切换**：本地转换，不依赖网络
- **语音听写（可选插件）**：本机识别，不外发音频
- **个人词库**：选词自学习；词库管理、黑白名单、导入导出——百分百本机自主管理
- **开放 API（可选）**：默认关闭；开启后仅监听本机 `127.0.0.1`，供本机 AI Agent / 脚本按习惯协助配置输入法

## 隐私

严格隐私保护，完全离线无账号：

- 键入内容、候选与词库只保存在本机，不上传输入正文
- 无账号登录、无行为画像、不出售输入数据
- 输入法本身不主动发起任何网络请求
- 开放 API 默认关闭；开启后也不对外网暴露

## 仓库结构（简述）

| 目录 | 说明 |
|------|------|
| `prisIragent_ime/` | Windows TSF 外壳与服务（含开放 API） |
| `prisir_ime/` | 同源 Rust 引擎 |
| `voice_input/` | 可选语音听写组件 |
| `installer/` | 安装包构建脚本与舞台资源 |

## 安装

从官网下载最新 Setup 安装包，按向导完成；在 Windows「语言 → 中文 → 选项 → 键盘」中启用灵犀。

开发者打安装包可参考 `installer/` 目录内脚本说明。

## 开放 API（可选）

面向「用 AI 帮助用户定制个性化输入法」：本机 Agent 可查询状态、调整配置、访问词库相关能力。

```text
prisir_tsfsvc --open-api-config enable    # 需要时开启
prisir_tsfsvc --open-api-config disable   # 关闭
prisir_tsfsvc --about openapi             # 说明
```

默认关闭；仅本机回环，不外发数据。

## 相关链接

- 官网介绍与安装包：https://www.babelspan.com/prisir-ime.html
- Android 仓：https://github.com/63894696/lingxi-android
- Linux 仓：https://github.com/63894696/lingxi-linux
- 问题反馈：https://bbs.babelspan.com/forum.html#board=browser/ime&hint=prisirai

版权所有 © 2026 BabelSpan。

## License

**GNU Affero General Public License v3.0 or later** — see [LICENSE](LICENSE).

Attribution is required, and derivative works must remain open source, including
when the software is only offered as a network service (AGPL §13). What is
covered, what keeps its own license, and the commercial-exception path are
described in [LICENSING.md](LICENSING.md).
