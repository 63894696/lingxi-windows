# 构建与发布（灵犀拼音输入法 · Windows）

> **唯一入口**：`bash installer/build_release.sh`
> 目标：干净工作区一条命令 → 一个可校验的安装包。**不允许**再手工 `cp` + `makensis`。

---

## 1. 一条命令

```bash
# 全流程：引擎 → 外壳 → stage 组装 → 校验 → NSIS → 安装包校验
bash installer/build_release.sh

# 升版后打包（只需改这一处版本号，其余全部生成）
bash installer/build_release.sh --bump beta.26

# 只到 stage（装/测用，不压缩，约 1 分钟）
bash installer/build_release.sh --no-nsis

# 复用已编译产物，只重新组装+打包
bash installer/build_release.sh --skip-build
```

前置：Rust（MSVC toolchain）、Python 3、NSIS（`C:/Program Files (x86)/NSIS/makensis.exe`）、7-Zip。
MSVC 环境经 `installer/_cargo_release.bat` 桥接（Git Bash 调 cmd 时嵌套引号会被吃掉）。

## 2. 版本号单一来源

`prisiragent_ime/VERSION.txt` 的 `PRIMARY_VERSION` 是唯一来源：

| 位置 | 角色 |
|---|---|
| `prisiragent_ime/VERSION.txt` | **源**（改这里） |
| `installer/stage/VERSION.txt` | 生成（构建脚本复制，已 gitignore） |
| NSIS `VERSION` define | 由脚本 `/DVERSION=` 传入（脚本内默认值只作兜底） |
| 根 `VERSION.txt` | Android 端版本号来源（独立，别混） |

历史坑：三处各写一个版本号 → "安装包写 beta.24、源码还写 beta.9"。

## 3. 产物与命名

| 产物 | 来源 crate | stage 名 | 说明 |
|---|---|---|---|
| `prisir_ime.dll` | `prisir_ime` | 同名 | 引擎，37 个 C 导出 |
| `prisiragent_ime.dll` | `prisiragent_ime` | **`prisir_ime_tsf.dll`** | TSF COM 外壳，13 个导出（5 COM + 8 `prisir_tsf_*`） |
| `prisir_tsfsvc.exe` | `prisiragent_ime` | 同名 | 控制壳（--register/--daemon/--status…） |
| `prisir_hw.exe` | `prisiragent_ime`（`--features hw`） | 同名 | 手写识别独立进程（ort 运行时加载 onnxruntime） |

> 文件名 `prisir_ime_tsf.dll` **不能改**：注册表 `InprocServer32` 与 NSIS 都按这个名字取。
> 2026-11 的事故就出在这：crate 改名后产物叫 `prisiragent_ime.dll`，脚本仍拷旧名，
> 结果 beta.21 之前的安装包里 TSF DLL 一直是 9 月的旧文件。

## 4. 资源（不入库，必须在本机就位）

| 资源 | 权威路径 | 备注 |
|---|---|---|
| 词库 `ciku.db` | `installer/stage/models/ciku.db`（174MB） | phrase 187 万 + pinyin 33 万 + wubi86 13.7 万；含 `word_blacklist`/`word_whitelist` 表（**必须预置**，只读连接上建表会让引擎返回 NULL → 整只输入法哑掉） |
| 词库备份 | `installer/stage/models/ciku.db.bak_*` | `.bak_jpfix`（jp 修复前）、`.bak_before_terms`（术语入库前） |
| 语音 `lingxi_voice.exe` + SenseVoice 模型 | `installer/stage/voice/` | 模型 `model.onnx`/`tokens.json` 走 gitignore |
| Python 前端 .py（外挂/诊断用） | `${LINGXIME_PY_SRC:-~/voice_input/lingxi_ime}` | 构建时复制进 `stage/lingxi_ime/`，仓库不存副本（避免第二份真相） |

## 5. 校验清单（脚本自动跑，任一失败即中止）

1. **导出表**：引擎 37 个导出中 14 个关键项齐全；TSF DLL 4 个 COM 项齐全
2. **产物新鲜度**：引擎/外壳产物 mtime 必须 < 24h（防"打旧包"）
3. **stage 一致性**：4 个二进制逐个 md5 与 `target/release` 一致
4. **词库体检**：`phrase > 1,000,000` 且黑白名单表存在（只读打开，绝不写）
5. **安装包**：> 250MB、PE 头、`7z t` 完整通过
6. **三端冒烟矩阵**：`installer/smoke_matrix.py`（用例见 `prisir_ime/abi_cases.json`）
   Windows 与 Linux 各跑一遍，结果必须逐条一致；**矩阵不过不许打包**
7. **词库索引**：`installer/db_indexes.py` 幂等补两个索引（缺了不会错，只是慢）
   · `idx_phrase_key_tail3` — 反向混拼（`sjie`）末 3 字过滤
   · `idx_phrase_value` — 词库窗按词前缀搜索（原来子串扫 3.1 秒）

手工补验（脚本无法代劳）：

```bash
python installer/check_release.py exports installer/stage/prisir_ime.dll
python installer/check_release.py stage   installer/stage
python installer/check_release.py exe     installer/LingxiIME-*-Setup.exe
```

## 6. 当前已发布的包

| 版本 | 文件 | 大小 | 说明 |
|---|---|---|---|
| beta.24 | `LingxiIME-Windows-x64-1.0.0-beta.24-beta-Setup.exe` | 311,994,632 B | P0/P1/P2 全量（手工打包） |
| beta.25 | `LingxiIME-Windows-x64-1.0.0-beta.25-beta-Setup.exe` | 312,743,282 B | 纯工程化版本：内容同 beta.24，由单一入口重建（`prisir_hw.exe` 重新编译） |
| beta.26 | `LingxiIME-Windows-x64-1.0.0-beta.26-beta-Setup.exe` | 341,800,055 B | 新词库：22 个领域来源新增 49,938 词 + 抢救 981,421 行 `key='100'` 死词 + `term_domain` 235k；词库 174MB→247MB |
| beta.27 | `LingxiIME-Windows-x64-1.0.0-beta.27-beta-Setup.exe` | 341,927,043 B | 引擎排序：精确全拼命中优先（`duomotai → 多模态`）+ 模糊音候选不再被截断 |
| beta.28 | `LingxiIME-Windows-x64-1.0.0-beta.28-beta-Setup.exe` | 341,882,578 B | 词库窗接入黑/白名单：「屏蔽 / 置顶 / 解除」+ 行标记 `[禁]/[顶]` |
| beta.29 | `LingxiIME-Windows-x64-1.0.0-beta.29-beta-Setup.exe` | 345,813,141 B | 反向混拼提速 341→35~60ms（`length(jp)` 约束 + `substr(key,-3)` 表达式索引） |

> **2026-11-17 起暂停出包**（用户决定："安装包等完整开发结束再打包"）。此后 stage 保持最新，
> 词库已含 `idx_phrase_value`（314MB）；恢复打包时直接 `bash installer/build_release.sh --bump <ver>`。

## 7. 已知缺口（写清楚，别让它悄悄回归）

| 缺口 | 现状 | 影响 | 处置 |
|---|---|---|---|
| **IMM32 / Win10 搜索框** | 2026-09-04 起 IMM 导出从 TSF DLL 拆走（双导出单文件会被 SearchApp 拒激活），但 `imm_only` 构建**尚未接线**（`build.rs` 只链 TSF def） | 当前包只有 TSF：普通 Win32/Electron/Everything 可用；Win10 开始菜单搜索框等 UWP 沉浸壳**打不了字** | 若要恢复：`--features imm_only` + `prisir_ime_imm.def` 链出 `.ime`，用 `check_release.py exports <ime> --require-imm` 校验 |
| `prisir_hw.exe` | 需 `--features hw` 才会编（依赖 ort 2.0.0-rc） | 不编则手写功能失效 | 已在脚本里固定 `--features hw` |
| 三份代码副本 | `~/oi_enhancements`（唯一活跃）/ `D:/Projects/lingxi-ime`（旧）/ `~/prisir_ime_wsl`（旧） | 用旧副本构建 = 打出旧包 | 旧副本已标 `DEPRECATED.md`，构建脚本写死 `ROOT=仓库根` |
| Python 前端 | 停在 2026-08-19，未同步 P0/P1/P2 | 只影响历史诊断前端 `app_debug.py`，打包仍带 | 定位为"历史诊断工具"，或后续改为直接调 FFI |

## 8. 验证纪律

- **TSF 改动只在 VM 里验**：host 不注册任何 TSF/IMM 组件（一次 explorer 崩溃就够了）。
- **无假 PASS**：不接受"返回 1 / 看起来对"，必须真值断言（`zhrmghg → 中华人民共和国`、Everything 真打字）。
- **装完自检**：`prisir_tsfsvc.exe --version`、记事本打 `nihao` + 空格、状态栏七键可点。
