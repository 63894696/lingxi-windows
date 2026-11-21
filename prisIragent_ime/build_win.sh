#!/usr/bin/env bash
# -*- coding: utf-8 -*-
#
# build_win.sh — 灵犀拼音输入法 Win 端打包脚本(对齐 Android 端 build.sh 流程)
#
# 流程:
#   1) 读 VERSION.txt(单一来源)
#   2) cargo build --release
#   3) 凑出 <version>-<channel>/ 文件夹: prisir_tsfsvc.exe + prisir_ime_tsf.dll + prisir_ime.dll + VERSION.txt + ABOUT.md 等
#   4) zip 压缩成 <product>-Windows-x64-<version>-<channel>.zip
#   5) 复制到 installer/_dist/windows/<channel>/
#   6) 调 python installer/build_win_manifest.py 生成 manifest.json + checksums.sha256
#
# 产物命名(对齐 Android 端):
#   LingxiIME-Windows-x64-<PRIMARY_VERSION>-<RELEASE_CHANNEL>.zip
#
# 用法:
#   bash build_win.sh                # 默认 channel=beta, VERSION.txt 决定
#   bash build_win.sh --no-zip       # 只构建 release, 不打 zip(本地验证用)
#   bash build_win.sh --channel beta # 显式指定通道
#
set -euo pipefail

# ===== 路径定位 =====
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
VERSION_FILE="${SCRIPT_DIR}/VERSION.txt"
TARGET_DIR="${SCRIPT_DIR}/target/release"
DIST_ROOT="${REPO_ROOT}/installer/_dist/windows"
MANIFEST_SCRIPT="${REPO_ROOT}/installer/build_win_manifest.py"

# ===== 参数 =====
NO_ZIP=0
FORCE_CHANNEL=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-zip)       NO_ZIP=1; shift ;;
        --channel)      FORCE_CHANNEL="${2:-}"; shift 2 ;;
        --help|-h)
            echo "用法: bash build_win.sh [--no-zip] [--channel <stable|beta|dev>]"
            exit 0
            ;;
        *) echo "[build_win] 未知参数: $1" >&2; exit 1 ;;
    esac
done

# ===== 1) 读 VERSION.txt =====
if [[ ! -f "${VERSION_FILE}" ]]; then
    echo "[build_win] VERSION.txt 缺失: ${VERSION_FILE}" >&2
    exit 1
fi
# shellcheck disable=SC1090
source "${VERSION_FILE}"   # 读 PRIMARY_VERSION / RELEASE_CHANNEL / BUILD_DATE / GIT_COMMIT

if [[ -n "${FORCE_CHANNEL}" ]]; then
    RELEASE_CHANNEL="${FORCE_CHANNEL}"
fi

PRODUCT="LingxiIME"
ARCH="x64"
ZIP_BASENAME="${PRODUCT}-Windows-${ARCH}-${PRIMARY_VERSION}-${RELEASE_CHANNEL}"

echo "=== VERSION: ${PRIMARY_VERSION} (${RELEASE_CHANNEL}) date=${BUILD_DATE} ==="
echo "=== target dir: ${TARGET_DIR} ==="

# ===== 2) cargo build =====
cd "${SCRIPT_DIR}"
cargo build --release --bin prisir_tsfsvc 2>&1 | tail -5

# ===== 3) 凑产物目录 =====
STAGE_DIR="${TARGET_DIR}/${ZIP_BASENAME}"
rm -rf "${STAGE_DIR}"
mkdir -p "${STAGE_DIR}"

# 3 个核心 dll/exe
for f in prisir_tsfsvc.exe prisir_ime_tsf.dll prisir_ime.dll; do
    if [[ -f "${TARGET_DIR}/${f}" ]]; then
        cp -f "${TARGET_DIR}/${f}" "${STAGE_DIR}/"
    else
        echo "[build_win] 警告: 产物缺失 ${f}, 跳过(继续打 zip)" >&2
    fi
done

# ===== 词库打进安装包(用户要求"安装即用") =====
# ciku.db + ciku.idx 是核心词库, 用户拷了 zip 解压后立即能打字
# 默认从 voice_input/lingxi_ime/backend/ 拷(开发机权威源);
# 也可 LINGXIME_CIKU_DIR=<路径> 覆盖, 用于 CI 从 artifacts 拉取
DEFAULT_CIKU_DIR="${REPO_ROOT}/../voice_input/lingxi_ime/backend"
CIKU_DIR="${LINGXIME_CIKU_DIR:-${DEFAULT_CIKU_DIR}}"
mkdir -p "${STAGE_DIR}/models"
CIKU_OK=1
for f in ciku.db ciku.idx; do
    if [[ -f "${CIKU_DIR}/${f}" ]]; then
        cp -f "${CIKU_DIR}/${f}" "${STAGE_DIR}/models/${f}"
        echo "[build_win] 模型文件已打包: ${f} ($(du -h "${CIKU_DIR}/${f}" | cut -f1))"
    else
        echo "[build_win] 警告: 词库文件缺失 ${CIKU_DIR}/${f}" >&2
        CIKU_OK=0
    fi
done
if [[ ${CIKU_OK} -eq 0 ]]; then
    echo "[build_win] 警告: 词库缺失, 装包后用户还需手动拷 ciku.db / ciku.idx, 失去'安装即用'能力" >&2
    echo "[build_win] 警告: 提示: LINGXIME_CIKU_DIR=/path/to/ciku bash build_win.sh 覆盖" >&2
fi

# VERSION.txt(单一来源, 跟随安装包走)
cp -f "${VERSION_FILE}" "${STAGE_DIR}/VERSION.txt"

# 说明页 markdown（关于 / 隐私政策 / 使用条款 / 开放 API / 问题反馈）
cat > "${STAGE_DIR}/ABOUT.md" <<'ABOUT_EOF'
# 灵犀拼音输入法 for Windows — 关于

灵犀拼音输入法（Windows）由 BabelSpan · Prisir（湃睿思）提供。

产品定位：面向个人与专业场景的本地中文输入工具。核心能力包括拼音/混拼候选、整句解码、简繁输出、本地词库与用户词典管理，以及可选的语音听写插件。输入链路在本机完成，不依赖账号体系，也不以云端联想为默认能力。

若需借助 AI Agent 按个人习惯定制输入法（词库、模式、查询等），请参阅「开放 API」：默认关闭，按需开启后仅对本机回环地址开放。

官网与下载：https://www.babelspan.com/

版权所有 © 2026 BabelSpan。本软件依据 Apache License, Version 2.0 授权，按「原样」提供，不附带任何担保或条件。完整授权见安装目录 LICENSE.txt（Apache License 2.0）。

## 隐私政策

灵犀拼音采用严格的隐私保护设计，完全离线无账号运行：

- 键入内容、候选结果与词库数据保存在本机，不向第三方上传输入正文。
- 不提供账号登录，不做行为画像，不出售或交换个人输入数据。
- 用户可通过词库管理与本地学习记录自主维护本地词典。
- 可选的语音听写在本机完成识别，不外发音频。
- 开放 API 默认关闭；开启后也仅监听本机 127.0.0.1，不对外网暴露。

输入法本身不主动发起任何网络请求。

## 使用条款

使用本软件即表示您知悉并同意以下约定：

- 本软件为本地工具，安装、配置与使用风险由用户自行承担。
- 请仅将本软件用于合法用途；请勿用于干扰系统安全或侵犯他人权益的场景。
- 词库与学习数据归用户本地管理；卸载前请自行备份需要保留的用户词典。
- 软件可能通过官网发布更新；是否安装更新由用户决定。

完整法律文本以安装目录 LICENSE.txt（Apache License, Version 2.0）为准。官网：https://www.babelspan.com/

## 开放 API

开放 API 面向「用 AI 帮助用户定制个性化输入法」的场景：本机 Agent / 脚本可通过本地接口查询状态、调整配置、访问词库相关能力，从而按个人习惯协助设置输入法。

默认关闭。需要 Agent 接入时，先开启：

```cmd
prisir_tsfsvc --open-api-config enable
```

开启后仅监听本机 127.0.0.1（默认端口 23331），不对外网开放。配置文件：`%LOCALAPPDATA%\Prisir\IME\open_api.json`。

常用端点：`GET /health` · `GET /status` · `POST /rpc`（JSON-RPC）。

## 问题反馈

如需报告缺陷或提出改进建议，请通过 BabelSpan 论坛提交：

https://bbs.babelspan.com/forum.html#board=browser/ime&hint=prisirai

建议说明：Windows 版本与灵犀版本号、复现步骤、期望与实际结果。
ABOUT_EOF

# 安装步骤(沿用 .tmp_p3j_t9_install/PrisirIME_v0.7.0/INSTALL.md,版本号改对齐)
cat > "${STAGE_DIR}/INSTALL.md" <<INSTALL_EOF
# 灵犀拼音输入法 for Windows v${PRIMARY_VERSION} (${RELEASE_CHANNEL}) — 安装步骤

## 包内容

\`\`\`
${ZIP_BASENAME}/
├── prisir_tsfsvc.exe       控制壳 (--register / --unregister / --enable / --disable / --daemon / --status / --version / --about <key> / --ipc / --ipc-test / --method <pinyin|wubi>)
├── prisir_ime_tsf.dll      TSF COM 库 (ITfTextInputProcessor + ITextStoreACP)
├── prisir_ime.dll          拼音查询引擎 (FFI 给 TSF DLL 调)
├── models/
│   ├── ciku.db             162 MB, 词库 (SQLite)
│   └── ciku.idx            214 MB, 索引 (拼音声母 → 词条)
├── VERSION.txt             版本号单一来源
├── ABOUT.md                关于/隐私政策/使用条款/问题反馈
└── INSTALL.md              本文件
\`\`\`

### 1. 装包 (管理员 cmd)

zip 内已含词库 (models/ciku.db 162MB + models/ciku.idx 214MB), 解压后 xcopy 一步搞定:

\`\`\`cmd
mkdir "C:\Program Files\PrisirIME"
xcopy /E /I /Y <解压目录>\\* "C:\Program Files\PrisirIME\\"
\`\`\`

(<解压目录> = 实际解压出来的 \`LingxiIME-Windows-x64-1.0.0-beta.1-beta\` 目录绝对路径,
xcopy /E 递归 /I 当目标是文件不存在时建目录 /Y 不提示覆盖)

## 2. 注册输入法

\`\`\`cmd
cd "C:\Program Files\PrisirIME"
prisir_tsfsvc.exe --register
\`\`\`

## 3. 重启 explorer.exe (关键 — Windows CTF 缓存)

\`\`\`cmd
taskkill /F /IM explorer.exe
start explorer.exe
\`\`\`

## 4. 切到灵犀

- \`Win+Space\` 切换输入法 → 选「Prisir 输入法」
- 或 \`右Ctrl\` (拼音默认激活键) → 直接进入拼音模式

## 5. 启动 daemon (新开管理员 cmd)

\`\`\`cmd
cd "C:\Program Files\PrisirIME"
prisir_tsfsvc.exe --daemon
\`\`\`

## 6. 端到端验证

打开记事本 (\`notepad.exe\`),按 \`n\` \`i\` \`h\` \`a\` \`o\` \`空格\`,期望:候选窗出现 → 「你好」上屏。

## 7. 查关于页

\`\`\`cmd
prisir_tsfsvc.exe --version
prisir_tsfsvc.exe --about about
prisir_tsfsvc.exe --about privacy
prisir_tsfsvc.exe --about terms
prisir_tsfsvc.exe --about contact
\`\`\`

## 8. 清理 (测试完必跑)

\`\`\`cmd
cd "C:\Program Files\PrisirIME"
prisir_tsfsvc.exe --unregister
taskkill /F /IM explorer.exe
start explorer.exe
\`\`\`

## 红线 (不污染 HKLM)

- Prisir IME 只走 HKCU, 不需要管理员注册 (但 \`copy\` 到 \`C:\Program Files\` 还是需要管理员)
- 测试完必 \`--unregister\` 清理
- \`ciku.db\` 不要拷到 HKCU profile 下, 装到 \`C:\Program Files\PrisirIME\models\` (per-machine)
INSTALL_EOF

echo "=== 阶段目录: ${STAGE_DIR} ==="
ls -la "${STAGE_DIR}/"

# ===== 4) 打 zip =====
ZIP_PATH="${TARGET_DIR}/${ZIP_BASENAME}.zip"
if [[ ${NO_ZIP} -eq 0 ]]; then
    cd "${TARGET_DIR}"
    rm -f "${ZIP_PATH}"
    # zip 用 7z 兜底 (Windows 上 `zip` 命令可能缺); 优先用系统 zip
    if command -v zip >/dev/null 2>&1; then
        zip -r "${ZIP_PATH}" "${ZIP_BASENAME}" >/dev/null
    else
        # powershell Compress-Archive (Windows 默认有)
        # 路径要用 Windows 风格 (C:\...) 不能用 Git Bash 的 /c/...
        WIN_STAGE=$(cygpath -w "${STAGE_DIR}")
        WIN_ZIP=$(cygpath -w "${ZIP_PATH}")
        powershell.exe -NoProfile -Command "Compress-Archive -Path '${WIN_STAGE}' -DestinationPath '${WIN_ZIP}' -Force" >/dev/null
    fi
    echo "=== zip 产物: ${ZIP_PATH} ($(du -h "${ZIP_PATH}" | cut -f1)) ==="
fi

# ===== 5) 复制到分发目录 =====
DIST_DIR="${DIST_ROOT}/${RELEASE_CHANNEL}"
mkdir -p "${DIST_DIR}"
if [[ -f "${ZIP_PATH}" ]]; then
    cp -f "${ZIP_PATH}" "${DIST_DIR}/"
fi
cp -rf "${STAGE_DIR}" "${DIST_DIR}/"  # 同时保留未压缩的目录,给 ad-hoc 装机用

# ===== 6) 生成 manifest =====
if [[ -f "${MANIFEST_SCRIPT}" ]]; then
    echo "=== 调 build_win_manifest.py ==="
    WIN_MANIFEST=$(cygpath -w "${MANIFEST_SCRIPT}")
    python "${WIN_MANIFEST}" --channel "${RELEASE_CHANNEL}" || true
else
    echo "[build_win] 警告: ${MANIFEST_SCRIPT} 不存在, 跳过 manifest 生成" >&2
fi

echo "=== DONE ==="
echo "产物落点:"
echo "  - ${TARGET_DIR}/${ZIP_BASENAME}.zip"
echo "  - ${DIST_DIR}/"