# 灵犀拼音 · Windows

本地中文拼音输入法（Windows TSF）。与 [Android](https://github.com/63894696/lingxi-android)、[Linux](https://github.com/63894696/lingxi-linux) 共用同一套 Rust 引擎，候选、整句与词库学习行为一致。

**官网下载**：https://www.babelspan.com/prisir-ime.html  
**许可证**：GNU AGPL-3.0-or-later（见 [LICENSE](LICENSE)）

## 功能概览

- **拼音输入**：全拼 / 混拼候选、整句解码、学习上屏
- **简繁切换**：本地转换，不依赖网络
- **语音听写（可选插件）**：本机识别，不外发音频
- **个人词库**：选词自学习；词库管理、导入导出——百分百本机自主管理
- **灵犀造词**：手工加词并指定候选位（全拼主行 + 简拼别名）
- **开放 API（可选）**：默认关闭；开启后仅监听本机 `127.0.0.1`，供本机 AI Agent / 脚本按习惯协助配置输入法

## 隐私

严格隐私保护，完全离线无账号：

- 键入内容、候选与词库只保存在本机，不上传输入正文
- 无账号登录、无行为画像、不出售输入数据
- 输入法本身不主动发起任何网络请求
- 开放 API 默认关闭；开启后也不对外网暴露

## 仓库结构（简述）

| 路径 | 说明 |
|------|------|
| `prisir_ime/` | 共享 Rust 引擎（候选 / 词库 / 学习） |
| `prisIragent_ime/` | Windows TSF 外壳与状态栏 |
| `installer/` | NSIS 安装包脚本与舞台文件 |
| `voice_input/` | 可选语音听写插件 |

## 构建（开发者）

详见 [`installer/BUILD.md`](installer/BUILD.md)。日常用户请从官网下载安装包，无需自行编译。

## 许可证

本仓库以 **GNU Affero General Public License v3.0 or later** 发布，详见 [LICENSE](LICENSE)、[LICENSING.md](LICENSING.md)、[NOTICE](NOTICE)。
