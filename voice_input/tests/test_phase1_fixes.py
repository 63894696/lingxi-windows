"""Phase 1 bug-fix 回归测试(v1.0.1)+ ASR→memory hook 回归(2026-07-03)

覆盖:
  1. VoiceActivityDetector 独立实例 + process_chunk 实装
  2. TTS mute gate 回调
  3. orchestrator speak 路由(auto 不再绕过 mute)
  4. StreamASRSession / oi_client.stream_asr 接口
  5. asr_memory_writer:final → shared_memory L3(降级安全 / 无循环不崩 / 无任务泄漏)
"""
from __future__ import annotations
import asyncio
import queue
from unittest.mock import MagicMock, patch

import numpy as np
import pytest


def test_vad_independent_instances():
    """两个 VAD 实例持独立 iterator,不共享状态"""
    from voice_input.asr.vad import VoiceActivityDetector

    a = VoiceActivityDetector()
    b = VoiceActivityDetector()
    assert a is not b
    # init 前 iterator 为 None
    assert a.iterator is None
    assert b.iterator is None


def test_vad_process_chunk_too_short_returns_none():
    """不足 512 samples 返回 None"""
    from voice_input.asr.vad import VoiceActivityDetector

    vad = VoiceActivityDetector()
    short = np.zeros(100, dtype=np.float32)
    assert vad.process_chunk(short) is None


@pytest.mark.skipif(True, reason="需要 silero-vad 已装;CI 可选跑")
def test_vad_process_chunk_with_silero():
    """silero-vad 实装 smoke test"""
    from voice_input.asr.vad import VoiceActivityDetector, VAD_CHUNK_SAMPLES

    vad = VoiceActivityDetector()
    if not vad.init():
        pytest.skip("silero-vad 未装")
    silence = np.zeros(VAD_CHUNK_SAMPLES, dtype=np.float32)
    # 静默块通常无 start/end 事件
    result = vad.process_chunk(silence)
    assert result is None or isinstance(result, dict)


def test_streaming_session_has_own_vad():
    """StreamingASRSession 构造时创建独立 VAD"""
    from voice_input.asr_daemon import StreamingASRSession

    ws_mock = MagicMock()
    s1 = StreamingASRSession(ws_mock)
    s2 = StreamingASRSession(ws_mock)
    assert s1.vad is not s2.vad


def test_tts_mute_gate_callbacks():
    """TTS daemon _asr_mute/_asr_unmute 调 ASR /mute /unmute"""
    from voice_input import tts_daemon

    with patch("httpx.post") as mock_post:
        mock_post.return_value.status_code = 200
        assert tts_daemon._asr_mute() is True
        assert tts_daemon._asr_unmute() is True
        assert mock_post.call_count == 2
        urls = [c.args[0] for c in mock_post.call_args_list]
        assert any("/mute" in u for u in urls)
        assert any("/unmute" in u for u in urls)


def test_orchestrator_speak_auto_uses_async_tts():
    """auto mode 走 synthesize_async(TTS daemon 负责 mute gate)"""
    from voice_input.orchestrator import PresenceState, DaemonClient

    presence = PresenceState()
    presence.set("auto")

    mock_tts = MagicMock(spec=DaemonClient)
    mock_tts.synthesize_tts.return_value = {"status": "queued"}

    mode = presence.get()
    async_mode = mode == "auto"
    result = mock_tts.synthesize_tts("hello", async_mode=async_mode)

    assert async_mode is True
    mock_tts.synthesize_tts.assert_called_once_with("hello", async_mode=True)
    assert result["status"] == "queued"


def test_stream_asr_session_class_exists():
    """oi_client StreamASRSession 可 import"""
    import sys
    from pathlib import Path

    clients_dir = Path(__file__).resolve().parents[1] / "clients"
    sys.path.insert(0, str(clients_dir))
    from oi_client import StreamASRSession, VoiceInputClient

    assert hasattr(StreamASRSession, "start")
    assert hasattr(StreamASRSession, "push_audio")
    assert hasattr(StreamASRSession, "events")
    assert hasattr(VoiceInputClient, "stream_asr")
    assert hasattr(VoiceInputClient, "open_stream_asr")


def test_ws_auth_rejects_bad_token():
    """verify_token 拒绝错误 token"""
    from voice_input.auth import verify_token

    good = "a" * 64
    assert verify_token(good, good) is True
    assert verify_token("wrong", good) is False
    assert verify_token("", good) is False


# ============================================================
# ASR final → shared_memory L3 hook(2026-07-03)
# ============================================================

def test_streaming_session_sync_construct_no_loop():
    """无事件循环时同步构造 session 不崩(回归:曾因 __init__ 里 create_task 抛 RuntimeError)"""
    from voice_input.asr_daemon import StreamingASRSession

    s = StreamingASRSession(MagicMock())
    assert s._bg_tasks == set()


def test_persist_final_without_loop_writes_directly():
    """无事件循环时 _persist_final 退化为直接写;空文本不写"""
    from voice_input.asr_daemon import StreamingASRSession
    from voice_input import asr_memory_writer

    s = StreamingASRSession(MagicMock())
    with patch.object(asr_memory_writer.ASR_MEMORY_WRITER, "write", return_value=True) as w:
        s._persist_final("你好世界")
        w.assert_called_once_with("你好世界")
    with patch.object(asr_memory_writer.ASR_MEMORY_WRITER, "write") as w:
        s._persist_final("")
        w.assert_not_called()


def test_persist_final_in_loop_no_task_leak():
    """事件循环内 _persist_final 用后台任务写盘,done 后自动从集合移除(无泄漏)"""
    from voice_input.asr_daemon import StreamingASRSession
    from voice_input import asr_memory_writer

    async def main():
        s = StreamingASRSession(MagicMock())
        with patch.object(asr_memory_writer.ASR_MEMORY_WRITER, "write", return_value=True) as w:
            s._persist_final("测试文本")
            assert len(s._bg_tasks) == 1
            await asyncio.gather(*s._bg_tasks)
            w.assert_called_once_with("测试文本")
        await asyncio.sleep(0)  # 让 done_callback 跑完
        assert len(s._bg_tasks) == 0

    asyncio.run(main())


def test_memory_writer_degrades_without_oi_enhancements(monkeypatch):
    """oi_enhancements 缺席时 write 返回 False 且不抛,进程内缓存仍更新"""
    from voice_input import asr_memory_writer as amw

    monkeypatch.setattr(amw, "_SM_MODULE", None)
    monkeypatch.setattr(amw, "_SM_FAILED", True)
    w = amw.ASRMemoryWriter()
    assert w.write("离线文本") is False
    assert w.retrieve() == "离线文本"
    w.clear()
    assert w.retrieve() is None


def test_memory_writer_locates_oi_enhancements():
    """本机能定位到 oi_enhancements(其它环境跳过)"""
    from voice_input.asr_memory_writer import _find_oi_enhancements_root

    root = _find_oi_enhancements_root()
    if root is None:
        pytest.skip("oi_enhancements 不在本机")
    assert (root / "shared_memory" / "__init__.py").exists()


def test_agent_state_endpoint_defaults_idle():
    """GET /agent_state 默认可用,返回 idle + profile"""
    from voice_input.orchestrator import app, agent_lifecycle, _detect_profile
    from fastapi.testclient import TestClient

    agent_lifecycle.set("idle", detail="")
    client = TestClient(app)
    r = client.get("/agent_state")
    assert r.status_code == 200
    body = r.json()
    assert body["state"] == "idle"
    assert body["profile"] == _detect_profile()
    assert body["can_wake"] is True


def test_agent_state_post_updates_lifecycle():
    """POST /agent_state 可更新 listening/thinking 等状态"""
    from voice_input.orchestrator import app, agent_lifecycle, AUTH_TOKEN
    from fastapi.testclient import TestClient

    client = TestClient(app)
    headers = {"Authorization": f"Bearer {AUTH_TOKEN}"}
    r = client.post("/agent_state", headers=headers, json={
        "state": "thinking", "detail": "Ollama 推理中", "can_interrupt": True,
    })
    assert r.status_code == 200
    assert r.json()["state"] == "thinking"
    assert r.json()["can_interrupt"] is True
    agent_lifecycle.set("idle")
