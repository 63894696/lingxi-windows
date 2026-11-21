"""voice_input 基础版 e2e 测试(2026-07-02 ship)

测试目标:3 daemon 起来 + 走通完整流程(无需真模型)

跑法(需先装 deps):
  cd C:/Users/Administrator/voice_input
  pip install -e .[dev]
  pytest tests/ -v

注意:这些是骨架测试,完整实装待 v0.2 commit。
"""
from __future__ import annotations
import pytest


def test_imports():
    """所有子模块能 import(无语法错)"""
    from voice_input import config, auth
    from voice_input.asr import sensevoice, vad
    from voice_input.tts import sapi
    from voice_input.audio import capture, inject
    from voice_input.hotkey import keyboard_global
    assert config.__name__ == "voice_input.config"
    assert auth.__name__ == "voice_input.auth"


def test_auth_token_generation(tmp_path):
    """auth_token 生成 + 校验"""
    from voice_input.auth import generate_auth_token, load_auth_token, verify_token
    token_file = tmp_path / "token"
    token = generate_auth_token(token_file)
    assert len(token) == 64  # 256-bit hex
    assert token == load_auth_token(token_file)
    assert verify_token(token, token) is True
    assert verify_token("wrong", token) is False
    assert verify_token("", "") is False


def test_presence_cycles():
    """presence_mode 循环切"""
    from voice_input.orchestrator import PresenceState
    p = PresenceState()
    assert p.get() == "voice"  # 默认
    p.cycle()  # voice → text
    assert p.get() == "text"
    p.cycle()  # text → auto
    assert p.get() == "auto"
    p.cycle()  # auto → voice(循环)
    assert p.get() == "voice"


def test_config_paths():
    """路径配置合理"""
    from voice_input.config import (
        ORCHESTRATOR_PORT, ASR_DAEMON_PORT, TTS_DAEMON_PORT,
        BIND_HOST, PRESENCE_MODES,
    )
    assert ORCHESTRATOR_PORT == 8730
    assert ASR_DAEMON_PORT == 8731
    assert TTS_DAEMON_PORT == 8732
    assert BIND_HOST == "127.0.0.1"  # loopback only
    assert PRESENCE_MODES == ("voice", "text", "auto")