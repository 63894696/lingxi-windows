import ctypes, time

so = "/root/prisir_ime_build/target/release/libprisir_ime.so"
lib = ctypes.CDLL(so)
lib.prisir_ime_load.restype = ctypes.c_void_p
lib.prisir_ime_load.argtypes = [ctypes.c_char_p, ctypes.c_int]
lib.prisir_ime_classify_with.restype = ctypes.c_int
lib.prisir_ime_classify_with.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
lib.prisir_ime_classify.restype = ctypes.c_int
lib.prisir_ime_classify.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
lib.prisir_ime_punct.restype = ctypes.c_void_p
lib.prisir_ime_punct.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int]
lib.prisir_ime_punct_reset.restype = None
lib.prisir_ime_punct_reset.argtypes = [ctypes.c_void_p]
lib.prisir_ime_query.restype = ctypes.c_void_p
lib.prisir_ime_query.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
lib.prisir_ime_free_string.restype = None
lib.prisir_ime_free_string.argtypes = [ctypes.c_void_p]


def take(p):
    if not p:
        return None
    s = ctypes.string_at(p)
    lib.prisir_ime_free_string(p)
    return s


t = time.time()
h = lib.prisir_ime_load(b"/usr/lib/fcitx5/prisir/ciku.db", 0)
assert h, "engine load failed"
print("engine loaded in %.1fs" % (time.time() - t))

KINDS = {0: "Empty", 1: "Pinyin", 2: "English", 3: "Ambiguous"}
E, P, EN, A = 0, 1, 2, 3

# (输入, 期望判定, 说明)
cases = [
    ("",         E,  "空缓冲"),
    ("s",        A,  "单字母 → 存疑，保持拼音候选"),
    ("z",        A,  "单字母 → 存疑"),
    ("nihao",    P,  "全拼"),
    ("shijie",   P,  "全拼"),
    ("mingong",  P,  "全拼（回溯切分边界）"),
    ("shurufa",  P,  "全拼"),
    ("women",    P,  "全拼"),
    ("ai",       P,  "全拼（单音节）"),
    ("an",       P,  "全拼（单音节）"),
    ("sjie",     P,  "混拼 s+jie，词库精确命中"),
    ("shij",     P,  "混拼 shi+j，词库精确命中"),
    ("hello",    EN, "英文：可切音节但词库无精确命中"),
    ("world",    EN, "英文"),
    ("python",   EN, "英文"),
    ("github",   EN, "英文"),
    ("docker",   EN, "英文"),
    # 下面三例**不是**误判:词库里确有同形简拼,尊重词库(用户可选到中文词)
    ("xyz",      P,  "真实简拼 xi-ya-zi → 细信子"),
    ("hhh",      P,  "真实简拼 ha-ha-ha → 哈哈哈"),
    ("ok",       P,  "真实简拼 o-u-ka → 欧卡"),
    ("test",     P,  "搜车：jp=test → 托尔斯泰（已知局限）"),
]

print("\n=== 中英文自动判定（真实词库，精确命中判据）===")
fail = 0
for s, exp, why in cases:
    q = take(lib.prisir_ime_query(h, s.encode())) if s else None
    n = q.count(b'"word"') if q else 0
    k1 = lib.prisir_ime_classify(h, s.encode())
    k2 = lib.prisir_ime_classify_with(h, s.encode(), -1)
    good = (k1 == exp and k2 == exp)
    if not good:
        fail += 1
    print("  %s %-10r 判定=%-9s 期望=%-9s 候选数=%3d  %s" % (
        "OK " if good else "FAIL", s, KINDS[k1], KINDS[exp], n, why))
print("  结果: %s" % ("全部符合预期" if fail == 0 else "%d 条不符合预期" % fail))

print("\n=== 智能标点 ===")
lib.prisir_ime_punct_reset(h)
seq = [",", ".", "?", "!", "\\", "<", ">", "^", "_", "$", "~", "(", ")", "[", "]", ";", ":"]
expect = {",": "，", ".": "。", "?": "？", "!": "！", "\\": "、",
          "<": "《", ">": "》", "^": "……", "_": "——", "$": "￥", "~": "～",
          "(": "（", ")": "）", "[": "【", "]": "】", ";": "；", ":": "："}
allok = True
for ch in seq:
    got = take(lib.prisir_ime_punct(h, ord(ch), 0))
    exp = expect[ch].encode()
    if got != exp:
        allok = False
    print("  %s %-3r -> %-8r (期望 %r)" % ("OK " if got == exp else "FAIL", ch,
                                          got.decode() if got else None, expect[ch]))
print("  标点映射: %s" % ("全部正确" if allok else "有失败"))

print("\n=== 引号配对 ===")
lib.prisir_ime_punct_reset(h)
got = []
for _ in range(3):
    r = take(lib.prisir_ime_punct(h, ord('"'), 0))
    got.append(r.decode() if r else None)
print("  三次 \" -> %s  %s" % (got, "OK" if got == ["“", "”", "“"] else "FAIL"))
lib.prisir_ime_punct_reset(h)
r = take(lib.prisir_ime_punct(h, ord('"'), 0))
print("  重置后 \" -> %r  %s" % (r.decode() if r else None,
                                 "OK" if (r and r.decode() == "“") else "FAIL"))

print("\n=== 未映射键放行 ===")
ok = True
for ch in ["a", " ", "@", "+", "=", "/", "#"]:
    r = take(lib.prisir_ime_punct(h, ord(ch), 0))
    if r is not None:
        ok = False
    print("  %s %-3r -> %r" % ("OK " if r is None else "FAIL", ch,
                               r.decode() if r else None))
print("  放行: %s" % ("全部正确" if ok else "有失败"))

print("\n结论: %s" % ("全部通过" if fail == 0 and allok and ok else "存在失败项"))
