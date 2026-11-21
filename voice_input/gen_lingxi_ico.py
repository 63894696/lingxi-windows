# -*- coding: utf-8 -*-
"""从 1024 精致原图一步缩小,生成多尺寸 .ico + 高清 PNG

Windows 桌面若只有 16×16,系统硬拉伸会糊成像素点。
必须用大图 LANCZOS 缩小并写入 16/32/48/64/128/256。
"""
from pathlib import Path
from PIL import Image, ImageDraw

ROOT = Path(__file__).parent
SRC = ROOT / "icon_candidates" / "lingxi_leaf_mic.png"
OUT_ICO = ROOT / "lingxi.ico"
OUT_PNG = ROOT / "lingxi.png"


def round_from(src: Image.Image, size: int) -> Image.Image:
    m = src.resize((size, size), Image.Resampling.LANCZOS)
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).ellipse([0, 0, size - 1, size - 1], fill=255)
    out = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    out.paste(m, (0, 0), mask)
    return out


def main():
    src = Image.open(SRC).convert("RGBA")
    if min(src.size) < 512:
        raise SystemExit(f"源图太小 {src.size},需要大图才能缩小保精致")
    sizes = [(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
    big = round_from(src, 256)
    # Pillow 官方多尺寸写法:以最大图为基准,sizes= 让其再生成各档
    big.save(OUT_ICO, format="ICO", sizes=sizes)
    big.save(OUT_PNG, format="PNG")
    raw = OUT_ICO.read_bytes()
    count = int.from_bytes(raw[4:6], "little")
    print(f"ICO {OUT_ICO} ({OUT_ICO.stat().st_size} bytes) entries={count}")
    for i in range(count):
        o = 6 + i * 16
        w, h = raw[o], raw[o + 1]
        nbytes = int.from_bytes(raw[o + 8 : o + 12], "little")
        print(f"  {w or 256}x{h or 256}  {nbytes} bytes")
    print(f"PNG {OUT_PNG} ({OUT_PNG.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
