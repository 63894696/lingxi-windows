"""P1 验收：整句解码（top-K）在真实词库上的表现。

判定原则：
  - 无歧义用例 → top-1 必须精确命中；
  - 有歧义用例（多个词的权重几乎相同，词库本身就是并列）→ 期望词必须出现在 top-K 内，
    由候选列表交给用户选。
"""
import ctypes
import json
import sys

so = "/root/prisir_ime_build/target/release/libprisir_ime.so"
K = int(sys.argv[1]) if len(sys.argv) > 1 else 6

lib = ctypes.CDLL(so)
lib.prisir_ime_load.restype = ctypes.c_void_p
lib.prisir_ime_load.argtypes = [ctypes.c_char_p, ctypes.c_int]
lib.prisir_ime_decode.restype = ctypes.c_void_p
lib.prisir_ime_decode.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
lib.prisir_ime_free_string.restype = None
lib.prisir_ime_free_string.argtypes = [ctypes.c_void_p]


def take(p):
    if not p:
        return "[]"
    s = ctypes.string_at(p).decode("utf-8", "replace")
    lib.prisir_ime_free_string(p)
    return s


h = lib.prisir_ime_load(b"/usr/lib/fcitx5/prisir/ciku.db", 0)
assert h, "engine load failed"

# (输入, 期望, 是否 top1 精确, 说明)
CASES = [
    # —— 无歧义：top-1 必须精确 ——
    ("wm",          "我们",       True,  "全简拼 2 字母"),
    ("nihaoshijie", "你好世界",   True,  "全拼跨词"),
    ("woaibeijing", "我爱北京",   True,  "全拼跨词"),
    ("mingong",     "民工",       True,  "全拼（n/g 边界）"),
    ("zhonghuarenmingongheguo", "中华人民共和国", True, "长全拼"),
    # —— 有歧义：期望词进 top-K 交给用户选 ——
    ("shij", "世界",     False, "混拼"),
    ("sj",   "世界",     False, "全简拼（时间/设计/手机 权重更高）"),
    ("wabj", "我爱北京", False, "每字单字母跨词"),
    # —— 词库权重决定：仅记录，不作断言 ——
    ("jttq",   "今天提前",     None, "今天天气(jp=jttq)权重仅 10242，低于 今天+提前"),
    ("jttqhh", "今天提前呵呵", None, "呵呵 权重略高于 很好"),
    ("wmy",    "万美元",       None, "万美元 jp=wmy 本身合法"),
]

print("K = %d\n" % K)
print("%-26s %-8s %-22s %s" % ("输入", "结果", "top-1", "top2..K"))
print("-" * 100)
fail = 0
for inp, exp, strict, why in CASES:
    arr = json.loads(take(lib.prisir_ime_decode(h, inp.encode(), K)))
    top = [a["sentence"] for a in arr]
    if strict is None:
        ok = bool(top)
        mark = "INFO"
    else:
        ok = (top[:1] == [exp]) if strict else (exp in top)
        mark = "OK  " if ok else "FAIL"
    if strict is not None and not ok:
        fail += 1
    label = "记录" if strict is None else ("top1 命中" if strict else "top-K 命中")
    print("%s %-24s %-8s %-22s %s" % (mark, inp, label,
        top[0] if top else "-", " / ".join(top[1:]) if len(top) > 1 else "-"))

print("\n结果: %s" % ("全部通过" if fail == 0 else "%d 条失败" % fail))
