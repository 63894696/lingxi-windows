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
# 版本号单一来源: prisiragent_ime/VERSION.txt (PRIMARY_VERSION)
#
# 为什么每一步都要断言（都咬过一次）:
#   1) crate 改名后产物不再是 prisir_ime_tsf.dll → 曾把 9 月的旧 DLL 打进 beta.21 之前的包
#   2) stage 与构建产物不是同一份 md5 → 装完发现新功能全没有
#   3) 引擎只读连接上建表 → 引擎返回 NULL，整只输入法哑掉
#   4) 缺 ImeGetImeMenuItems → ImmInstallIME 返 0x715，输入法装不上
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENGINE_DIR="${ROOT}/prisir_ime"
SHELL_DIR="${ROOT}/prisiragent_ime"
STAGE="${ROOT}/installer/stage"
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
# Windows Python 吃盘符路径；Git Bash 的 /d/... 会被错译成 D:\d\...（与下方 py_win 同因）
py_win() { python "$(cygpath -w "$1")" "${@:2}"; }
if [[ -n "${BUMP}" ]]; then
  # --bump 支持两种写法：完整 semver（1.0.0-beta.25）或只给后缀（beta.25，沿用现有 MAJOR.MINOR.PATCH）
  py_win "${ROOT}/installer/set_version.py" "$(cygpath -w "${VERSION_FILE}")" "${BUMP}"
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
  say "3/6 编译外壳 prisiragent_ime（lib + tsfsvc + hw）"
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
# 另外：产物 mtime 必须不早于关键源文件（防 cargo 桥静默未重编却仍 exit 0）
if [[ ${SKIP_BUILD} -eq 0 ]]; then
  now=$(date +%s)
  for f in "${ENGINE_DLL}" "${SHELL_DLL}" "${SVC}"; do
    age=$(( now - $(stat -c %Y "$f") ))
    (( age < 86400 )) || die "产物超过 24h，疑似旧文件: $f"
  done
  about_src="${SHELL_DIR}/src/about.rs"
  [[ -f "${about_src}" ]] || die "缺 ${about_src}"
  if [[ "$(stat -c %Y "${SHELL_DLL}")" -lt "$(stat -c %Y "${about_src}")" ]]; then
    die "外壳 DLL 比 about.rs 旧 — cargo 未真正重编（检查 _cargo_release.bat）"
  fi
  # 文案断言：新包必须带 AGPL + 新菜单标题，不得残留旧口语栏
  py_win "${ROOT}/installer/check_about_strings.py" "$(cygpath -w "${SHELL_DLL}")" \
    || die "外壳 DLL 文案断言失败（未打进 AGPL/新关于文案）"
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
# 协议：始终从 installer/LICENSE.txt 同步进 stage（NSIS 协议页读这份）
cp -f "${ROOT}/installer/LICENSE.txt" "${STAGE}/LICENSE.txt"
[[ -f "${STAGE}/ABOUT.md" ]] || die "缺 stage/ABOUT.md（安装后可读的关于全文）"
# 官方 AGPL 全文写的是 "GNU AFFERO GENERAL PUBLIC LICENSE"，通常不含字面量 "AGPL-3.0"
grep -qi "GNU AFFERO GENERAL PUBLIC LICENSE" "${STAGE}/LICENSE.txt" \
  || die "stage/LICENSE.txt 不是 AGPL（缺 GNU AFFERO GENERAL PUBLIC LICENSE）"
grep -q "AGPL-3.0" "${STAGE}/ABOUT.md" || die "stage/ABOUT.md 未声明 AGPL-3.0"

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
# Git Bash 下 ${DB} 是 /d/...；Windows Python sqlite URI 要盘符路径，否则 unable to open database file
DB_WIN="$(cygpath -w "${DB}")"
python - "${DB_WIN}" <<'PY'
import sqlite3, sys
c = sqlite3.connect(f'file:{sys.argv[1]}?mode=ro', uri=True)
n = c.execute('select count(*) from phrase').fetchone()[0]
t = {r[0] for r in c.execute("select name from sqlite_master where type='table'")}
assert n > 1_000_000, f'phrase 只有 {n} 行，词库不完整'
assert {'word_blacklist', 'word_whitelist'} <= t, '缺黑白名单表（只读连接上建表会让引擎返回 NULL）'
print(f'[db] phrase={n:,} 表齐全')
PY

# 词库索引（幂等）：反向混拼慢查询依赖 substr(key,-3) 表达式索引，缺了不会错，只是慢
py_win "${ROOT}/installer/db_indexes.py" --db "${DB_WIN}"

# ── 5) 校验（stage 一致性 + 导出表） ─────────────────────────────────────────
say "5/6 校验"
STAGE_WIN="$(cygpath -w "${STAGE}")"
py_win "${CHECK}" exports "${STAGE_WIN}\\prisir_ime.dll"
py_win "${CHECK}" exports "${STAGE_WIN}\\prisir_ime_tsf.dll"
# 若将来恢复 IMM 分包（SearchApp 需要），把产物放到 stage/prisir_ime_imm.ime 后开这行：
# py_win "${CHECK}" exports "${STAGE_WIN}\\prisir_ime_imm.ime" --require-imm
py_win "${CHECK}" stage "${STAGE_WIN}"

# 三端冒烟矩阵（同一份用例 Windows/Linux/Android 共用）：引擎行为回归的最后一道闸
py_win "${ROOT}/installer/smoke_matrix.py" --lib "${STAGE_WIN}\\prisir_ime.dll" --db "${DB_WIN}" \
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
( cd "${ROOT}/installer" && MSYS_NO_PATHCONV=1 "${MAKENSIS}" /DVERSION="${VERSION}" /DCHANNEL="${CHANNEL}" "${NSI_WIN}" ) \
  | tail -14
[[ -f "${ROOT}/installer/${OUT}" ]] || die "NSIS 未产出 ${OUT}"
py_win "${CHECK}" exe "$(cygpath -w "${ROOT}/installer/${OUT}")"
# 同步一份到 dist/，方便对外交付
mkdir -p "${ROOT}/dist"
cp -f "${ROOT}/installer/${OUT}" "${ROOT}/dist/${OUT}"
say "完成"
echo "安装包: installer/${OUT}  (${VERSION})"
echo "副本:   dist/${OUT}"
