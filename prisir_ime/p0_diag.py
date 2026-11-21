import ctypes, sqlite3, shutil, os, tempfile

SRC = "/usr/lib/fcitx5/prisir/ciku.db"
so = "/root/prisir_ime_build/target/release/libprisir_ime.so"
lib = ctypes.CDLL(so)
lib.prisir_ime_load.restype = ctypes.c_void_p
lib.prisir_ime_load.argtypes = [ctypes.c_char_p, ctypes.c_int]

print("=== 部署词库里的表 ===")
con = sqlite3.connect("file:%s?mode=ro" % SRC, uri=True)
tabs = [r[0] for r in con.execute("select name from sqlite_master where type='table'")]
con.close()
print(" ", tabs)
print("  有 word_blacklist:", "word_blacklist" in tabs)
print("  有 word_whitelist:", "word_whitelist" in tabs)

print("\n=== A) 原样加载（表缺失 + 只读） ===")
h = lib.prisir_ime_load(SRC.encode(), 0)
print("  handle =", h, "->", "加载成功" if h else "加载失败(复现)")

print("\n=== B) 复制到可写目录（同样表缺失） ===")
d = tempfile.mkdtemp()
p1 = os.path.join(d, "ciku_nodb.db")
shutil.copyfile(SRC, p1)
h = lib.prisir_ime_load(p1.encode(), 0)
print("  handle =", h, "->", "加载成功" if h else "加载失败")

print("\n=== C) 先补齐黑白名单表，再加载 ===")
p2 = os.path.join(d, "ciku_withdb.db")
shutil.copyfile(SRC, p2)
con = sqlite3.connect(p2)
con.executescript("""
CREATE TABLE IF NOT EXISTS word_blacklist (value TEXT PRIMARY KEY, reason TEXT, added_at TEXT DEFAULT (datetime('now')));
CREATE TABLE IF NOT EXISTS word_whitelist (value TEXT PRIMARY KEY, reason TEXT, added_at TEXT DEFAULT (datetime('now')));
""")
con.commit(); con.close()
h = lib.prisir_ime_load(p2.encode(), 0)
print("  handle =", h, "->", "加载成功" if h else "加载失败")
print("\n结论: 加载失败的直接原因是只读连接上执行 CREATE TABLE。")
