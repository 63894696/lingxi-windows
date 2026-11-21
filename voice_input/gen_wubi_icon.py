# -*- coding: utf-8 -*-
"""用 agnesai 端点按 Prisir 铜光金属圆环视觉规范生成灵犀·五笔图标候选(2026-08-19)
与拼音"拼"字徽章同一视觉语言,主体换"五"字(五笔输入法)。"""
from __future__ import annotations
import base64, json, os, sys, urllib.request
from pathlib import Path

BASE_URL = os.environ.get("AGNES_BASE_URL", "https://apihub.agnes-ai.com/v1").rstrip("/")
API_KEY = os.environ.get("AGNES_API_KEY", "")
OUT = Path(r"C:\Users\Administrator\voice_input\icon_candidates")
OUT.mkdir(exist_ok=True)

STYLE = (
    "circular app icon, deep ink blue-black background (#0f1a24), "
    "a glowing copper metallic ring around the edge (copper #c98a4b) with radiant light flares and sparkling glints, "
    "bronze/copper metallic 3D emblem in the center, "
    "subtle glowing teal accent (#2f8f83), small luminous particles orbiting like fireflies, "
    "rich embossed religious-medallion quality, premium metallic sheen, high detail, square 1:1 icon"
)
SUBJECTS = {
    "wu_char": "an embossed copper Chinese character '五' (meaning five/wubi) as the central emblem, calligraphic metallic strokes, encircled by small radical root symbols",
    "wu_root": "a copper Chinese character '五' fused with five keyboard keycaps arranged around it as the central emblem (wubi input, five-stroke method)",
}


def gen(name, subject):
    prompt = f"{STYLE}, {SUBJECTS[subject]}, centered composition"
    body = {"model": "agnes-image-2.1-flash", "prompt": prompt, "n": 1, "size": "1024x1024"}
    req = urllib.request.Request(
        f"{BASE_URL}/images/generations",
        data=json.dumps(body).encode("utf-8"),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {API_KEY}"},
        method="POST")
    try:
        with urllib.request.urlopen(req, timeout=180) as r:
            data = json.loads(r.read().decode("utf-8"))
    except Exception as e:
        print(f"[{name}] 失败: {e}")
        return None
    item = (data.get("data") or [{}])[0]
    img_bytes = None
    if item.get("b64_json"):
        img_bytes = base64.b64decode(item["b64_json"])
    elif item.get("url"):
        with urllib.request.urlopen(item["url"], timeout=60) as ir:
            img_bytes = ir.read()
    if not img_bytes:
        print(f"[{name}] 无图像数据")
        return None
    path = OUT / f"wubi_{name}.png"
    path.write_bytes(img_bytes)
    print(f"[{name}] OK {path.name} ({len(img_bytes)//1024}KB)")
    return path


if not API_KEY:
    print("ERROR: AGNES_API_KEY 未设置", file=sys.stderr); sys.exit(1)
if __name__ == "__main__":
    for subj in (sys.argv[1:] or list(SUBJECTS)):
        gen(subj, subj)
    print("完成,候选在 icon_candidates/wubi_*.png")
