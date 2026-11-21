import ctypes

so = "/root/prisir_ime_build/target/release/libprisir_ime.so"
lib = ctypes.CDLL(so)
lib.prisir_ime_load.restype = ctypes.c_void_p
lib.prisir_ime_load.argtypes = [ctypes.c_char_p, ctypes.c_int]
lib.prisir_ime_smart_sentence.restype = ctypes.c_void_p
lib.prisir_ime_smart_sentence.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
lib.prisir_ime_query.restype = ctypes.c_void_p
lib.prisir_ime_query.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
lib.prisir_ime_free_string.restype = None
lib.prisir_ime_free_string.argtypes = [ctypes.c_void_p]


def take(p):
    if not p:
        return None
    s = ctypes.string_at(p).decode("utf-8", "replace")
    lib.prisir_ime_free_string(p)
    return s


h = lib.prisir_ime_load(b"/usr/lib/fcitx5/prisir/ciku.db", 0)
assert h

cases = [
    # (输入, 期望, 说明)
    ("sj",         "世界",           "全简拼 2 字"),
    ("shij",       "世界",           "混拼"),
    ("wmy",        "我们",           "全简拼 3 字母（当前是否支持？）"),
    ("wm",         "我们",           "全简拼 2 字母"),
    ("jttq",       "今天天气",       "全简拼 4 字母"),
    ("jttqhh",     "今天天气很好",   "全简拼 6 字母（跨词）"),
    ("nihaoshijie","你好世界",       "全拼跨词"),
    ("nihaoshijie","你好世界",       "全拼"),
    ("woaibeijing", "我爱北京",      "全拼跨词"),
    ("wabj",       "我爱北京",       "全简拼跨词"),
    ("mingong",    "民工",           "全拼（n/g 边界）"),
    ("zhonghuarenmingongheguo", "中华人民共和国", "长全拼"),
    ("zhrmghg",    "中华人民共和国", "超长简拼"),
]

print("%-28s %-16s %-16s %s" % ("输入", "当前 smart_sentence", "期望", "候选数"))
print("-" * 90)
for inp, exp, why in cases:
    got = take(lib.prisir_ime_smart_sentence(h, inp.encode()))
    q = take(lib.prisir_ime_query(h, inp.encode()))
    n = q.count('"word"') if q else 0
    mark = "OK  " if got == exp else "MISS"
    print("%s %-24s %-16s %-16s %3d   %s" % (mark, inp, got or "-", exp, n, why))
