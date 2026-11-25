#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""灵犀输入法装包校验 — 导出表 / stage 一致性 / 安装包完整性。

为什么要有这个脚本：beta.22 之前，所有安装包里的 TSF DLL 都是 9 月的旧文件
（crate 改名后产物叫 prisiragent_ime.dll，脚本仍拷 prisir_ime_tsf.dll）。
"看起来打包成功"是最贵的假 PASS，所以打包后必须逐项验：

  python installer/check_release.py exports  installer/stage/prisir_ime.dll
  python installer/check_release.py exports  installer/stage/prisir_ime_tsf.dll
  python installer/check_release.py stage    installer/stage
  python installer/check_release.py exe      installer/LingxiIME-...-Setup.exe

任一失败 → 退出码 1。
"""
import argparse
import hashlib
import os
import struct
import subprocess
import sys

# 引擎 ABL 关键导出：少一个就可能是旧 DLL（只查"该有的都在"，不锁总数上限）
ENGINE_REQUIRED = [
    "prisir_ime_load", "prisir_ime_query", "prisir_ime_query_wubi",
    "prisir_ime_smart_sentence", "prisir_ime_decode", "prisir_ime_sentences",
    "prisir_ime_classify", "prisir_ime_punct", "prisir_ime_set_trad",
    "prisir_ime_to_trad", "prisir_ime_to_script", "prisir_ime_user_add",
    "prisir_ime_dict_search_page", "prisir_ime_add_blacklist",
]
# TSF DLL 关键项：COM 入口 5 个（当前发布形态）。
# 2026-09-04 起 IMM32 导出已从本文件拆走（双导出单文件会被 SearchApp 拒激活），
# 所以这里不再要求 ImeXxx；若将来恢复 IMM 分包，用 --require-imm 单独校验。
TSF_REQUIRED = [
    "DllGetClassObject", "DllRegisterServer",
    "DllUnregisterServer", "DllCanUnloadNow",
]
# 纯 IMM 引擎（prisir_ime_imm.ime / --require-imm 时校验）：
# 16 个 ImeXxx 少一个就可能 ImmInstallIME 0x715 或加载后无候选。
IMM_REQUIRED = [
    "ImeInquire", "ImeSelect", "ImeProcessKey", "ImeToAsciiEx",
    "NotifyIME", "ImeConfigure", "ImeEscape", "ImeDestroy",
    "ImeSetActiveContext", "ImeConversionList", "ImeSetCompositionString",
    "ImeRegisterWord", "ImeUnregisterWord", "ImeGetRegisterWordStyle",
    "ImeEnumRegisterWord", "ImeGetImeMenuItems",
]
# stage 里必须与构建产物一致的二进制（名字 → 期望来源）
STAGE_BINARIES = {
    "prisir_ime.dll": "prisir_ime/target/release/prisir_ime.dll",
    "prisir_ime_tsf.dll": "prisiragent_ime/target/release/prisiragent_ime.dll",
    "prisir_tsfsvc.exe": "prisiragent_ime/target/release/prisir_tsfsvc.exe",
    "prisir_hw.exe": "prisiragent_ime/target/release/prisir_hw.exe",
}


def pe_exports(path):
    """最小 PE 导出表解析（不依赖 pefile / dumpbin）。"""
    d = open(path, "rb").read()
    pe = struct.unpack_from("<I", d, 0x3C)[0]
    if d[pe:pe + 4] != b"PE\0\0":
        raise SystemExit(f"[x] 不是 PE 文件: {path}")
    nsec = struct.unpack_from("<H", d, pe + 6)[0]
    optsz = struct.unpack_from("<H", d, pe + 20)[0]
    opt = pe + 24
    magic = struct.unpack_from("<H", d, opt)[0]
    dd = opt + (112 if magic == 0x20B else 96)
    exp_rva = struct.unpack_from("<I", d, dd)[0]
    if not exp_rva:
        return []
    secs = []
    for i in range(nsec):
        o = opt + optsz + i * 40
        vs, va, rs, ra = struct.unpack_from("<IIII", d, o + 8)
        secs.append((va, max(vs, rs), ra))

    def r2o(rva):
        for va, sz, ra in secs:
            if va <= rva < va + sz:
                return ra + (rva - va)
        return None

    eo = r2o(exp_rva)
    nnames = struct.unpack_from("<I", d, eo + 24)[0]
    npo = r2o(struct.unpack_from("<I", d, eo + 32)[0])
    out = []
    for i in range(nnames):
        no = r2o(struct.unpack_from("<I", d, npo + i * 4)[0])
        out.append(d[no:d.index(b"\0", no)].decode("ascii", "replace"))
    return out


def md5(path):
    h = hashlib.md5()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def cmd_exports(a):
    names = pe_exports(a.dll)
    if a.require_imm:
        required = IMM_REQUIRED
    elif "prisir_ime.dll" in a.dll:
        required = ENGINE_REQUIRED
    else:
        required = TSF_REQUIRED
    missing = [n for n in required if n not in names]
    print(f"[exports] {a.dll}: {len(names)} 个导出")
    if missing:
        print(f"[x] 缺关键导出: {missing}")
        return 1
    print(f"[✓] 关键导出齐全（{len(required)} 项）")
    return 0


def cmd_stage(a):
    root = a.stage
    bad = []
    for name, rel in a.binary.items():
        p = os.path.join(root, name)
        if not os.path.exists(p):
            bad.append(f"{name}: stage 缺失")
            continue
        src = rel if os.path.isabs(rel) else os.path.join(a.repo, rel)
        if not os.path.exists(src):
            bad.append(f"{name}: 构建产物缺失 {src}")
            continue
        if md5(p) != md5(src):
            bad.append(f"{name}: stage 与构建产物 md5 不一致")
        else:
            print(f"[✓] {name} == {rel} ({os.path.getsize(p):,} B)")
    for need in ("VERSION.txt", "models/ciku.db"):
        if not os.path.exists(os.path.join(root, need)):
            bad.append(f"{need}: 缺失")
    if bad:
        print("[x] stage 校验失败:")
        for b in bad:
            print("   -", b)
        return 1
    print("[✓] stage 一致性通过")
    return 0


def cmd_exe(a):
    p = a.exe
    if not os.path.exists(p):
        print(f"[x] 安装包不存在: {p}")
        return 1
    size = os.path.getsize(p)
    with open(p, "rb") as f:
        head = f.read(2)
    print(f"[exe] {os.path.basename(p)}: {size:,} B  sha256={hashlib.sha256(open(p,'rb').read()).hexdigest()}")
    if size < 250 * 1024 * 1024:
        print("[x] 体积异常（<250MB，词库/语音模型可能没打进去）")
        return 1
    if head != b"MZ":
        print("[x] 不是 PE 安装包")
        return 1
    z7 = r"C:\Program Files\7-Zip\7z.exe"
    if os.path.exists(z7):
        r = subprocess.run([z7, "t", p], capture_output=True, text=True,
                           encoding="utf-8", errors="replace")
        tail = (r.stdout or "").strip().splitlines()[-3:]
        print("[7z]", " | ".join(t.strip() for t in tail))
        if r.returncode != 0:
            print("[x] 7z 完整性校验失败")
            return 1
    print("[✓] 安装包校验通过")
    return 0


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("exports")
    p.add_argument("dll")
    p.add_argument("--require-imm", action="store_true",
                   help="按 IMM32 16 个导出校验（用于 .ime / imm_only 构建）")
    p.set_defaults(fn=cmd_exports)

    p = sub.add_parser("stage")
    p.add_argument("stage")
    p.add_argument("--repo", default=os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    p.add_argument("--binary", nargs="*", default=[])
    p.set_defaults(fn=cmd_stage)

    p = sub.add_parser("exe")
    p.add_argument("exe")
    p.set_defaults(fn=cmd_exe)

    a = ap.parse_args()
    if a.cmd == "stage":
        # 允许 --binary name=rel 覆盖默认映射
        m = dict(STAGE_BINARIES)
        for item in a.binary:
            k, v = item.split("=", 1)
            m[k] = v
        a.binary = m
    sys.exit(a.fn(a))


if __name__ == "__main__":
    main()
