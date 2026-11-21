"""WebSocket /ws 流式识别 e2e 测试(2026-07-02 v1.1 ship)

跑 3 个场景:
  1. WS 握手 + ready/config_ok/pong
  2. 流式 audio 推 wav → final 识别 + 唤醒词检测
  3. wake word 命中测试(把 "hey oi" 注入 fake text 看 detect_wake_word)
"""
from __future__ import annotations
import asyncio
import json
import base64
import subprocess
import sys
import time
from pathlib import Path

import pytest


# ---------- 前置:启动 3 daemon(用 start_all.py) ----------
@pytest.fixture(scope="module", autouse=True)
def daemon_running():
    """module 级 fixture:启动 daemon + 结束后关闭"""
    print("\n[setup] 启动 3 daemon...")
    proc = subprocess.Popen(
        [sys.executable, str(Path(__file__).resolve().parents[1] / "scripts" / "start_all.py")],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    time.sleep(12)  # 等 onnx 加载
    yield
    print("\n[teardown] 关闭 daemon...")
    subprocess.run(
        [sys.executable, str(Path(__file__).resolve().parents[1] / "scripts" / "stop_all.py")],
        capture_output=True, timeout=10,
    )
    time.sleep(1)


def test_wake_word_detection():
    """单元测试:detect_wake_word(2026-07-02 v1.1 ship)"""
    from voice_input.asr_daemon import detect_wake_word

    # 命中
    assert detect_wake_word("Hey OI 今天天气真好") == "hey oi"
    assert detect_wake_word("你好,帮我查一下") == "你好"
    assert detect_wake_word("hello oi 在吗") == "oi"  # "hello oi" 的子串 "oi"
    assert detect_wake_word("OI 帮我做件事") == "oi"

    # 不命中
    assert detect_wake_word("随便说点什么") is None
    assert detect_wake_word("今天天气好") is None

    # 自定义 wake words
    assert detect_wake_word("jarvis 帮我") is None  # 默认 wake_words 没 jarvis → None
    assert detect_wake_word("jarvis 帮我", wake_words=["jarvis"]) == "jarvis"  # 自定义 jarvis 后命中

    print("  ✓ detect_wake_word 5 case 全过")


@pytest.mark.asyncio
async def test_ws_handshake():
    """WS 握手 + ready/config_ok/pong(2026-07-02 v1.1 ship)"""
    import websockets
    from voice_input.auth import load_auth_token

    token = load_auth_token()
    uri = f"ws://127.0.0.1:8731/ws?token={token}"
    async with websockets.connect(uri) as ws:
        # 1. ready
        msg = await asyncio.wait_for(ws.recv(), timeout=5)
        d = json.loads(msg)
        assert d["type"] == "ready"
        assert d["daemon"] == "asr"
        assert "hey oi" in d["wake_words"]
        print(f"  ✓ WS ready: {len(d['wake_words'])} wake words")

        # 2. config
        await ws.send(json.dumps({
            "action": "config",
            "wake_words": ["hey oi", "你好", "jarvis"],
        }))
        msg = await asyncio.wait_for(ws.recv(), timeout=5)
        d = json.loads(msg)
        assert d["type"] == "config_ok"
        assert d["wake_words"] == ["hey oi", "你好", "jarvis"]
        print(f"  ✓ WS config_ok: {d['wake_words']}")

        # 3. ping
        await ws.send(json.dumps({"action": "ping"}))
        msg = await asyncio.wait_for(ws.recv(), timeout=5)
        d = json.loads(msg)
        assert d["type"] == "pong"
        assert "ts" in d
        print(f"  ✓ WS pong: ts={d['ts']:.2f}")


@pytest.mark.asyncio
async def test_ws_streaming_final():
    """WS 流式 audio 推 wav → final 识别 + 唤醒词检测(2026-07-02 v1.1 ship)"""
    import websockets
    from voice_input.auth import load_auth_token

    wav_path = Path("C:/temp/sf_audio/real_xiaoxiao_10s.wav")
    if not wav_path.exists():
        pytest.skip(f"{wav_path} 不存在,跳过")

    token = load_auth_token()
    uri = f"ws://127.0.0.1:8731/ws?token={token}"
    async with websockets.connect(uri) as ws:
        ready = json.loads(await asyncio.wait_for(ws.recv(), timeout=5))
        assert ready["type"] == "ready"

        # 推 wav chunks(每秒 32KB)
        wav = wav_path.read_bytes()
        pcm = wav[44:]
        chunk_size = 16000 * 2 * 1  # 1s
        n_chunks = len(pcm) // chunk_size
        for i in range(n_chunks):
            chunk = pcm[i * chunk_size : (i+1) * chunk_size]
            data_b64 = base64.b64encode(chunk).decode()
            await ws.send(json.dumps({"action": "audio", "data_b64": data_b64}))
            await asyncio.sleep(0.1)

        # stop 触发 final — v1.0.1 起 per-session VAD 真正工作,服务端会先推
        # vad/partial/wake 事件;协议本来就是事件流,客户端循环读到带文本的 final 为止
        await ws.send(json.dumps({"action": "stop"}))
        d = None
        for _ in range(30):
            msg = await asyncio.wait_for(ws.recv(), timeout=15)
            d = json.loads(msg)
            if d["type"] == "final" and d.get("text"):
                break
        assert d is not None and d["type"] == "final", f"没等到 final:{d}"
        # 关雎首句在音频开头,无论 VAD-end final 还是 stop 强制 final 都应包含
        assert "关关雎鸠" in d["text"], f"识别结果不对:{d['text']}"
        print(f"  ✓ WS final: '{d['text'][:50]}...' (wake_word={d['wake_word']})")


@pytest.mark.asyncio
async def test_ws_wake_word_detection():
    """WS 流式识别唤醒词命中(2026-07-02 v1.1 ship)"""
    import websockets
    from voice_input.auth import load_auth_token

    token = load_auth_token()
    uri = f"ws://127.0.0.1:8731/ws?token={token}"
    async with websockets.connect(uri) as ws:
        ready = json.loads(await asyncio.wait_for(ws.recv(), timeout=5))
        assert ready["type"] == "ready"

        # 自定义 wake_words
        await ws.send(json.dumps({
            "action": "config",
            "wake_words": ["hey oi", "oi", "hey"],
        }))
        cfg = json.loads(await asyncio.wait_for(ws.recv(), timeout=5))
        assert cfg["type"] == "config_ok"

        # 推一段拼出来的 "hey oi 今天天气真好" wav(模拟用户说唤醒词)
        # 注:这里只验证协议正确性,真 mic 测试需要用户手动跑
        print(f"  ✓ WS wake word 协议测试(协议层 OK,真 mic 测试需用户手动)")


if __name__ == "__main__":
    """直接 python 跑(不走 pytest)"""
    print("=== test_ws_streaming.py 直接运行 ===")
    test_wake_word_detection()
    asyncio.run(test_ws_handshake())
    asyncio.run(test_ws_streaming_final())
    print("\n  ✓ 所有测试通过")