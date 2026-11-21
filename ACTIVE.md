# 灵犀输入法 — 活跃工作区（2026-11-21）

三端**当前源码与打包**以 `D:\Projects` 为准：

| 端 | 路径 |
|---|---|
| Windows | `D:\Projects\lingxi-windows` |
| Android | `D:\Projects\lingxi-android` |
| Linux | `D:\Projects\lingxi-linux` |

## 例外（尚未整仓迁入 D:\Projects）

| 路径 | 角色 | 能否删 |
|---|---|---|
| `C:\Users\Administrator\voice_input` | Win 语音 `lingxi_voice.exe` 冻结权威源（PyInstaller + `_venv_voice`） | **否**（或先完整迁入再删） |
| `C:\Users\Administrator\oi_enhancements` | 旧「大仓」：含 `lexicon_pipeline`、历史 IME 副本、以及大量非输入法项目 | **不可整仓删**；IME 子树已落后于 `D:\Projects`，但词库管线仍在此 |
| `D:\Projects\lingxi-ime` | 已标 `DEPRECATED.md`（停在 11-15/16） | **可删**（确认无未备份的私货后） |
| `C:\Users\Administrator\prisir_ime_wsl` | 旧 WSL 引擎副本 | **可删**（确认无唯一产物后） |

`oi_enhancements` 里的 `DEPRECATED` 曾写「唯一活跃副本在 oi_enhancements」——**已过时**；自 2026-11-20 起以本文件与 `D:\Projects\lingxi-*` 为准。
