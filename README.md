# 灵犀拼音 · Windows 端工作区

迁自 `oi_enhancements`（2026-11-20），与 `D:\Projects\lingxi-android` 并列。

## 布局

| 路径 | 说明 |
|------|------|
| `prisIragent_ime/` | Windows TSF/IMM 外壳 + `prisir_tsfsvc`（含开放 API） |
| `prisir_ime/` | **目录联接** → `D:\Projects\lingxi-android\prisir_ime`（三端同源引擎） |
| `voice_input/` | 灵犀语音源码（PyInstaller → `lingxi_voice.exe`） |
| `installer/` | 构建脚本；`stage/` **联接** → oi_enhancements 侧资源（词库/语音，约 10GB） |
| ~~`fcitx5-prisir/`~~ | 已迁至 `D:\Projects\lingxi-linux` |

## 打 Windows 安装包

```bash
cd /d/Projects/lingxi-windows
bash installer/build_release.sh --bump beta.30
```

## 开放 API

用途：供本机 **AI Agent** 接入，协助用户定制个性化输入法（默认关闭，仅 `127.0.0.1`）。

```bash
prisir_tsfsvc --open-api-config enable   # 需要 Agent 接入时开启
prisir_tsfsvc --about openapi            # 说明文案
prisir_tsfsvc --open-api                 # 前台冒烟
```

配置：`%LOCALAPPDATA%\Prisir\IME\open_api.json`

## Linux (fcitx5) 功能补齐评估（2026-11-20）

结论：**值得做，且性价比高** — 核心输入能力并不落后，落后的是「产品壳」。

### 已具备（源码 `fcitx5-prisir`，~500 行 C++）
- 同源引擎 dlopen：`query / learn / classify / punct / sentences / to_script`
- P0 中英判定 + 智能标点；P1 整句候选；P2 简繁（Ctrl+Shift+F）
- Shift 临时英文；候选翻页；学习上屏

### 相对 Win/Android 缺口
| 缺口 | 难度 | 说明 |
|------|------|------|
| 词库管理 / 黑白名单 UI | 中 | 引擎 FFI 已有，缺 GTK/Qt 或配置面板 |
| 开放 API | 低 | 与 Win 同契约，daemon 起 127.0.0.1 |
| 九宫格 / 手写 | 高 | 手写模型分发受限；九宫格可接引擎 T9 |
| 设置页视觉对齐 | 中 | 新 Android 卡片风可做简易配置页 |
| 打包 `.deb` 升版 | 低 | 换新 `libprisir_ime.so` + 词库即可 |

### 建议路线（约 1–2 周可交付「可用升级」）
1. **P0（1–2 天）**：用当前 `prisir_ime` 重编 `.so` + 新词库打 `.deb`，跑 `smoke_matrix` Linux 侧
2. **P1（3–5 天）**：补开放 API + 简繁二态与 Android 对齐 + 基础配置（候选数/模糊音）
3. **P2（按需）**：词库窗；手写默认不做

引擎层几乎零改；工作量在外壳与打包验证。本机若无 Linux 构建机，可用 WSL2 或现有 VPS。
