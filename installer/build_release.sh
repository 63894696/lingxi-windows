#!/usr/bin/env bash
# -*- coding: utf-8 -*-
#
# build_release.sh — 灵犀拼音输入法 Windows 端唯一构建入口
#
# 目标：干净工作区一条命令 → 一个可校验的安装包。杜绝"打旧包"。
#
# 用法:
#   bash installer/build_release.sh                 # 全流程（引擎→外壳→stage→NSIS→校验）
#   bash installer/build_release.sh --bump beta.25  # 先改版本号再构建
#   bash installer/build_release.sh --no-nsis       # 只到 stage + 校验（快速回归）
#   bash installer/build_release.sh --skip-build    # 复用已编译产物，只重打包
#
# 版本号单一来源: prisIragent_ime/VERSION.txt (PRIMARY_VERSION)
#
# 为什么每一步都要断言（都咬过一次）:
#   1) crate 改名后产物不再是 prisir_ime_tsf.dll → 曾把 9 月的旧 DLL 打进 beta.21 之前的包
#   2) stage 与构建产物不是同一份 md5 → 装完发现新功能全没有
#   3) 引擎只读连接上建表 → 引擎返回 NULL，整只输入法哑掉
#   4) 缺 ImeGetImeMenuItems → ImmInstallIME 返 0x715，输入法装不上
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENGINE_DIR="${LINGXIME_ENGINE_DIR:-${ROOT}/prisir_ime}"
SHELL_DIR="${ROOT}/prisIragent_ime"
# STAGE 默认是 installer/stage(可能是 junction)。部分沙箱/IDE 环境禁止经 junction 穿越,
# 此时用 LINGXIME_STAGE 指定真实目录(如 C:/Users/<user>/oi_enhancements/installer/stage)。
STAGE="${LINGXIME_STAGE:-${ROOT}/installer/stage}"
VERSION_FILE="${SHELL_DIR}/VERSION.txt"
NSI="${ROOT}/installer/lingxi_ime.nsi"
CHECK="${ROOT}/installer/check_release.py"
MAKENSIS="${MAKENSIS:-C:/Program Files (x86)/NSIS/makensis.exe}"
# Python 前端权威源（不在本仓库，装包时复制进 stage）
PY_SRC="${LINGXIME_PY_SRC:-${HOME}/voice_input/lingxi_ime}"

NO_NSIS=0
SKIP_BUILD=0
BUMP=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-nsis)    NO_NSIS=1; shift ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    --bump)       BUMP="${2:-}"; shift 2 ;;
    -h|--help)    sed -n '3,25p' "$0"; exit 0 ;;
    *) echo "[build] 未知参数: $1" >&2; exit 1 ;;
  esac
done

say() { printf '\n=== %s ===\n' "$*"; }
die() { echo "[x] $*" >&2; exit 1; }
# Git Bash → Windows Python：把 /d/foo 转成 D:\foo，避免变成 D:\d\foo
winpath() { cygpath -w "$1"; }
# 调 Windows Python 时临时关掉路径自动转换
pyw() { MSYS_NO_PATHCONV=1 python "$@"; }

# ── 0) 工具链 ────────────────────────────────────────────────────────────────
say "0/6 工具链"
cargo --version
python --version 2>&1 | head -1
[[ -f "${MAKENSIS}" ]] || die "缺 makensis: ${MAKENSIS}"
# MSVC 链接器：cargo 需要在 vcvars 环境里才能 link。
# Git Bash → cmd 的嵌套引号会被吃掉，统一走 installer/_cargo_release.bat 桥接。
CARGO_BAT="${ROOT}/installer/_cargo_release.bat"
[[ -f "${CARGO_BAT}" ]] || die "缺构建桥: ${CARGO_BAT}"
cmd //c "$(cygpath -w "${CARGO_BAT}")" --where-link >/dev/null \
  || die "找不到 MSVC link.exe（vcvars64 失败）"

# ── 1) 版本号 ────────────────────────────────────────────────────────────────
say "1/6 版本号"
if [[ -n "${BUMP}" ]]; then
  # --bump 支持两种写法：完整 semver（1.0.0-beta.25）或只给后缀（beta.25，沿用现有 MAJOR.MINOR.PATCH）
  pyw "$(winpath "${ROOT}/installer/set_version.py")" "$(winpath "${VERSION_FILE}")" "${BUMP}"
fi
VERSION="$(grep -E '^PRIMARY_VERSION=' "${VERSION_FILE}" | cut -d= -f2)"
CHANNEL="$(grep -E '^RELEASE_CHANNEL=' "${VERSION_FILE}" | cut -d= -f2)"
[[ -n "${VERSION}" ]] || die "VERSION.txt 缺 PRIMARY_VERSION"
echo "[build] ${VERSION} (${CHANNEL})"

build_crate() {
  local dir="$1"; shift
  cmd //c "$(cygpath -w "${CARGO_BAT}")" "$(cygpath -w "${dir}")" "$@" \
    || die "cargo build 失败: ${dir} $*"
}

# ── 2) 引擎 ──────────────────────────────────────────────────────────────────
if [[ ${SKIP_BUILD} -eq 0 ]]; then
  say "2/6 编译引擎 prisir_ime"
  build_crate "${ENGINE_DIR}"
else
  say "2/6 跳过引擎编译（--skip-build）"
fi
ENGINE_DLL="${ENGINE_DIR}/target/release/prisir_ime.dll"
[[ -f "${ENGINE_DLL}" ]] || die "引擎产物缺失: ${ENGINE_DLL}"

# ── 3) TSF/IMM32 外壳 ────────────────────────────────────────────────────────
if [[ ${SKIP_BUILD} -eq 0 ]]; then
  say "3/6 编译外壳 prisIragent_ime（lib + tsfsvc + hw）"
  build_crate "${SHELL_DIR}"
  build_crate "${SHELL_DIR}" --bin prisir_tsfsvc
  build_crate "${SHELL_DIR}" --bin prisir_hw --features hw
else
  say "3/6 跳过外壳编译（--skip-build）"
fi
SHELL_DLL="${SHELL_DIR}/target/release/prisiragent_ime.dll"
SVC="${SHELL_DIR}/target/release/prisir_tsfsvc.exe"
HW="${SHELL_DIR}/target/release/prisir_hw.exe"
for f in "${SHELL_DLL}" "${SVC}" "${HW}"; do
  [[ -f "$f" ]] || die "外壳产物缺失: $f"
done

# 产物新鲜度：不允许打进 24 小时前的旧文件（这条正是"打旧包"事故的根因）
if [[ ${SKIP_BUILD} -eq 0 ]]; then
  now=$(date +%s)
  for f in "${ENGINE_DLL}" "${SHELL_DLL}" "${SVC}"; do
    age=$(( now - $(stat -c %Y "$f") ))
    (( age < 86400 )) || die "产物超过 24h，疑似旧文件: $f"
  done
fi

# ── 4) stage 组装 ────────────────────────────────────────────────────────────
say "4/6 组装 stage"
mkdir -p "${STAGE}/models" "${STAGE}/lingxi_ime/backend" "${STAGE}/lingxi_ime/frontend/pinyin"
# 名字必须保持 prisir_ime_tsf.dll（注册表 InprocServer32 + NSIS 都按这个名字取）
cp -f "${SHELL_DLL}" "${STAGE}/prisir_ime_tsf.dll"
cp -f "${ENGINE_DLL}" "${STAGE}/prisir_ime.dll"
cp -f "${SVC}" "${STAGE}/prisir_tsfsvc.exe"
cp -f "${HW}" "${STAGE}/prisir_hw.exe"
cp -f "${VERSION_FILE}" "${STAGE}/VERSION.txt"

# Python 前端（外挂/诊断用，历史遗留）——源在 LINGXIME_PY_SRC
if [[ -d "${PY_SRC}" ]]; then
  cp -f "${PY_SRC}/backend/database.py" "${PY_SRC}/backend/engine.py" "${STAGE}/lingxi_ime/backend/" 2>/dev/null || true
  cp -f "${PY_SRC}/frontend/pinyin/app_debug.py" "${STAGE}/lingxi_ime/frontend/pinyin/" 2>/dev/null || true
else
  echo "[!] 跳过 Python 前端复制（LINGXIME_PY_SRC=${PY_SRC} 不存在）"
fi

# 词库：stage/models/ciku.db 是权威源（含 jp 修复 + 黑白名单表 + 术语提权）。
# 缺了就装出一个"没词库的输入法"，必须硬失败。
DB="${STAGE}/models/ciku.db"
[[ -f "${DB}" ]] || die "词库缺失: ${DB}（从备份 .bak_* 恢复或跑 lexicon_pipeline）"
pyw - "$(winpath "${DB}")" <<'PY'
import sqlite3, sys, os
p = os.path.abspath(sys.argv[1])
c = sqlite3.connect(f'file:{p.replace(chr(92), "/")}?mode=ro', uri=True)
n = c.execute('select count(*) from phrase').fetchone()[0]
t = {r[0] for r in c.execute("select name from sqlite_master where type='table'")}
assert n > 1_000_000, f'phrase 只有 {n} 行，词库不完整'
assert {'word_blacklist', 'word_whitelist'} <= t, '缺黑白名单表（只读连接上建表会让引擎返回 NULL）'
print(f'[db] phrase={n:,} 表齐全')
PY

# 词库索引（幂等）：反向混拼慢查询依赖 substr(key,-3) 表达式索引，缺了不会错，只是慢
pyw "$(winpath "${ROOT}/installer/db_indexes.py")" --db "$(winpath "${DB}")"

# ── 5) 校验（stage 一致性 + 导出表） ─────────────────────────────────────────
say "5/6 校验"
pyw "$(winpath "${CHECK}")" exports "$(winpath "${STAGE}/prisir_ime.dll")"
pyw "$(winpath "${CHECK}")" exports "$(winpath "${STAGE}/prisir_ime_tsf.dll")"
# 若将来恢复 IMM 分包（SearchApp 需要），把产物放到 stage/prisir_ime_imm.ime 后开这行：
# pyw "$(winpath "${CHECK}")" exports "$(winpath "${STAGE}/prisir_ime_imm.ime")" --require-imm
pyw "$(winpath "${CHECK}")" stage "$(winpath "${STAGE}")" \
  --binary "prisir_ime.dll=$(cygpath -w "${ENGINE_DIR}/target/release/prisir_ime.dll")"

# 三端冒烟矩阵（同一份用例 Windows/Linux/Android 共用）：引擎行为回归的最后一道闸
pyw "$(winpath "${ROOT}/installer/smoke_matrix.py")" --lib "$(winpath "${STAGE}/prisir_ime.dll")" --db "$(winpath "${DB}")" \
  --cases "$(winpath "${ENGINE_DIR}/abi_cases.json")" \
  || die "ABI 冒烟矩阵未通过（引擎行为回归，别打包）"

# ── 6) NSIS ─────────────────────────────────────────────────────────────────
say "6/6 NSIS"
if [[ ${NO_NSIS} -eq 1 ]]; then
  echo "[build] --no-nsis，止步于 stage（可直接装/测）"
  exit 0
fi
OUT="LingxiIME-Windows-x64-${VERSION}-${CHANNEL}-Setup.exe"
# MSYS_NO_PATHCONV=1：否则 Git Bash 把 /DVERSION=... 当路径转成 C:/Program Files/Git/DVERSION
# 但关掉转换后，nsi 路径必须显式转成 Windows 形式，否则 makensis 把 /c/... 当成选项 → 打 Usage
NSI_WIN="$(cygpath -w "${NSI}")"
# SRC 必须用 Windows 绝对路径:NSI 里的 File "${SRC}\..." 在 makensis 当前目录下按相对路径解析,
# 经 junction 会被部分环境拒绝。
STAGE_WIN="$(cygpath -w "${STAGE}")"
( cd "${ROOT}/installer" && MSYS_NO_PATHCONV=1 "${MAKENSIS}" /DVERSION="${VERSION}" /DCHANNEL="${CHANNEL}" /DSRC="${STAGE_WIN}" "${NSI_WIN}" ) \
  | tail -14
[[ -f "${ROOT}/installer/${OUT}" ]] || die "NSIS 未产出 ${OUT}"
pyw "$(winpath "${CHECK}")" exe "$(winpath "${ROOT}/installer/${OUT}")"
say "完成"
echo "安装包: installer/${OUT}  (${VERSION})"
