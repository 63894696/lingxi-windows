"""OI 侧便捷客户端(ADR-001 §4 clients/)

OI 进程用这个 client 调 voice_input orchestrator 的 REST 接口。
不依赖 OI 内部 Python 版本,只依赖 httpx + websockets。

v0.4 新增一键方法:
  - record_and_transcribe() — 调 ASR daemon /start_listening → 自动 VAD 停 → 返回 wav bytes
  - transcribe_wav_bytes() — 上传 wav 到 ASR daemon /transcribe → 文本
  - inject_text() — 调 orchestrator /inject → 注入当前焦点窗口
  - speak_or_inject() — voice mode 走 TTS;text/auto mode 走 inject

v1.0.1 新增:
  - open_stream_asr() / StreamASRSession — 真 WebSocket 流式 ASR + token auth
"""
from __future__ import annotations
import asyncio
import base64
import json
import queue
import threading
import time
from pathlib import Path
from typing import Iterator, Optional
import httpx

from voice_input.config import (
    ORCHESTRATOR_PORT, ASR_DAEMON_PORT, BIND_HOST,
    AUTH_TOKEN_FILE, PRESENCE_MODES,
)


BASE_ORCH_URL = f"http://{BIND_HOST}:{ORCHESTRATOR_PORT}"
BASE_ASR_URL = f"http://{BIND_HOST}:{ASR_DAEMON_PORT}"
WS_ASR_URL = f"ws://{BIND_HOST}:{ASR_DAEMON_PORT}/ws"


class StreamASRSession:
    """WebSocket 流式 ASR session — 后台线程跑 asyncio,主线程 sync 推/收

    用法:
        session = client.open_stream_asr(wake_words=['hey oi'])
        session.start()
        session.push_wav_file('test.wav')
        for event in session.events():
            print(event)
        session.close()
    """

    def __init__(self, auth_token: str, wake_words: Optional[list] = None):
        self.auth_token = auth_token
        self.wake_words = wake_words
        self._uri = f"{WS_ASR_URL}?token={auth_token}"
        self._event_queue: queue.Queue = queue.Queue()
        self._cmd_queue: queue.Queue = queue.Queue()
        self._thread: Optional[threading.Thread] = None
        self._running = False
        self._ready = threading.Event()
        self._error: Optional[str] = None

    def start(self, timeout: float = 10.0):
        """启动后台 WS 连接"""
        if self._thread and self._thread.is_alive():
            return
        self._running = True
        self._thread = threading.Thread(target=self._run_async_loop, daemon=True)
        self._thread.start()
        if not self._ready.wait(timeout=timeout):
            raise TimeoutError("StreamASRSession 连接超时")

    def _run_async_loop(self):
        try:
            asyncio.run(self._async_main())
        except Exception as e:
            self._error = str(e)
            self._event_queue.put({"type": "error", "message": str(e)})
        finally:
            self._event_queue.put(None)

    async def _async_main(self):
        import websockets

        async with websockets.connect(self._uri) as ws:
            ready = json.loads(await ws.recv())
            self._event_queue.put(ready)
            self._ready.set()

            if self.wake_words:
                await ws.send(json.dumps({"action": "config", "wake_words": self.wake_words}))
                cfg = json.loads(await ws.recv())
                self._event_queue.put(cfg)

            async def receiver():
                while self._running:
                    try:
                        msg = await asyncio.wait_for(ws.recv(), timeout=0.5)
                        self._event_queue.put(json.loads(msg))
                    except asyncio.TimeoutError:
                        continue
                    except Exception:
                        break

            _SKIP = object()

            def _blocking_get():
                try:
                    return self._cmd_queue.get(timeout=0.2)
                except queue.Empty:
                    return _SKIP

            async def sender():
                loop = asyncio.get_running_loop()
                while self._running:
                    cmd = await loop.run_in_executor(None, _blocking_get)
                    if cmd is _SKIP:
                        continue
                    if cmd is None:
                        break
                    await ws.send(json.dumps(cmd))

            recv_task = asyncio.create_task(receiver())
            send_task = asyncio.create_task(sender())
            await asyncio.gather(recv_task, send_task, return_exceptions=True)

    def push_audio(self, pcm_bytes: bytes):
        """推送 PCM 或 WAV bytes"""
        self._cmd_queue.put({
            "action": "audio",
            "data_b64": base64.b64encode(pcm_bytes).decode(),
        })

    def push_wav_file(self, wav_path: str, chunk_seconds: float = 1.0):
        """分块推送 wav 文件(模拟流式)"""
        data = Path(wav_path).read_bytes()
        pcm = data[44:] if data[:4] == b"RIFF" else data
        chunk_size = int(16000 * 2 * chunk_seconds)
        for i in range(0, len(pcm), chunk_size):
            self.push_audio(pcm[i : i + chunk_size])
            time.sleep(0.05)

    def stop(self):
        """触发 final 识别"""
        self._cmd_queue.put({"action": "stop"})

    def ping(self):
        self._cmd_queue.put({"action": "ping"})

    def events(self, timeout: float = 30.0) -> Iterator[dict]:
        """Sync generator — yield WS 事件直到 None sentinel 或 timeout"""
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                remaining = max(0.1, deadline - time.time())
                event = self._event_queue.get(timeout=min(1.0, remaining))
            except queue.Empty:
                continue
            if event is None:
                break
            yield event
            if event.get("type") == "error":
                break

    def close(self):
        self._running = False
        self._cmd_queue.put(None)
        if self._thread and self._thread.is_alive():
            self._thread.join(timeout=3)


class VoiceInputClient:
    """OI 调 voice_input orchestrator 的便捷客户端(v0.4 升级)

    用法(在 OI chat 模式 LLM code block 里):
      from clients.oi_client import VoiceInputClient
      c = VoiceInputClient()

      # 健康 + 状态
      c.health()
      c.status()

      # 切 presence
      c.set_presence("voice")

      # 一键语音对话(mic 录音 → ASR → TTS)
      text = c.record_and_transcribe(max_seconds=15)   # 用户说话 → text
      c.speak(f"你说的是:{text}")                       # OI 回话

      # 一键注入(OI 输出直接进焦点窗口)
      c.inject_text("hello from OI")                    # text mode 友好

      # 一键 mic 录音 + ASR + inject(语音指令场景)
      text = c.record_and_transcribe()
      c.inject_text(text)
    """
    def __init__(
        self,
        base_url: Optional[str] = None,
        auth_token: Optional[str] = None,
    ):
        self.base_url = base_url or BASE_ORCH_URL
        self.asr_url = self.base_url.replace(str(ORCHESTRATOR_PORT), str(ASR_DAEMON_PORT))
        token = auth_token or self._load_token()
        self.headers = {"Authorization": f"Bearer {token}"}
        self.client = httpx.Client(timeout=30.0, headers=self.headers)

    def _load_token(self) -> str:
        token_file = Path(AUTH_TOKEN_FILE)
        if not token_file.exists():
            raise FileNotFoundError(
                f"auth_token 文件不存在:{token_file},先启动 orchestrator 生成"
            )
        return token_file.read_text(encoding="utf-8").strip()

    # ============================================================
    # 健康 / 状态 / presence
    # ============================================================
    def health(self) -> dict:
        """3 daemon 总状态"""
        r = self.client.get(f"{self.base_url}/health")
        r.raise_for_status()
        return r.json()

    def status(self) -> dict:
        """详细状态"""
        r = self.client.get(f"{self.base_url}/status")
        r.raise_for_status()
        return r.json()

    def get_presence(self) -> str:
        r = self.client.get(f"{self.base_url}/presence")
        r.raise_for_status()
        return r.json().get("mode", "auto")

    def set_presence(self, mode: str) -> str:
        """切 presence_mode"""
        if mode not in PRESENCE_MODES:
            raise ValueError(f"mode 必须是 {PRESENCE_MODES} 之一,不是 {mode}")
        r = self.client.post(f"{self.base_url}/presence", json={"mode": mode})
        r.raise_for_status()
        return r.json().get("mode", mode)

    # ============================================================
    # ASR — v0.4 新增(v0.2 SenseVoice 推理已在 daemon 端实装)
    # ============================================================
    def transcribe_wav_bytes(self, wav_bytes: bytes) -> dict:
        """上传 wav bytes 给 ASR daemon 识别(2026-07-02 v0.4 ship)

        Returns: dict {status, text, duration_ms, platform}
        """
        audio_b64 = base64.b64encode(wav_bytes).decode()
        r = self.client.post(
            f"{self.asr_url}/transcribe",
            json={"audio_b64": audio_b64},
        )
        r.raise_for_status()
        return r.json()

    def transcribe_wav_file(self, wav_path: str) -> dict:
        """从 wav 文件路径识别"""
        return self.transcribe_wav_bytes(Path(wav_path).read_bytes())

    # ============================================================
    # TTS / 注入 — v0.4 新增
    # ============================================================
    def speak(self, text: str) -> dict:
        """OI 提交文本 → orchestrator 按 presence_mode 路由

        - voice mode → 走 TTS(本地 SAPI),播放期间 mic mute
        - text  mode → 写 IM log 文件(基础版暂)
        - auto  mode → 异步 TTS(不阻塞)
        """
        r = self.client.post(f"{self.base_url}/speak", json={"text": text, "interrupt": True})
        r.raise_for_status()
        return r.json()

    def inject_text(self, text: str) -> dict:
        """OI 提交文本 → orchestrator 直接注入当前焦点窗口(2026-07-02 v0.4 ship)

        不走 TTS,直接 inject。适合 text mode / 命令行场景。
        配套 POST /inject 路由(orchestrator 加)。
        """
        r = self.client.post(f"{self.base_url}/inject", json={"text": text})
        r.raise_for_status()
        return r.json()

    # ============================================================
    # 一键组合方法 — v0.4 ship
    # ============================================================
    def record_and_transcribe(
        self,
        max_seconds: int = 15,
        input_device: Optional[int] = None,
        language: int = 1,
        textnorm: int = 2,
    ) -> dict:
        """一键 mic 录音 + ASR 返回文本(2026-07-02 v1.0 ship 完整)

        走 ASR daemon POST /record_and_transcribe:
          1. daemon mic VAD 录音(沉默自动停,最长 max_seconds)
          2. SenseVoice 转录 wav
          3. 返回 {text, duration_ms, wav_bytes}

        OI 实际使用:
          text = c.record_and_transcribe()['text']
          c.inject_text(f"你说了:{text}")
        """
        r = self.client.post(
            f"{self.asr_url}/record_and_transcribe",
            json={
                "max_seconds": max_seconds,
                "input_device": input_device,
                "language": language,
                "textnorm": textnorm,
            },
            timeout=max_seconds + 30,
        )
        r.raise_for_status()
        return r.json()

    def open_stream_asr(self, wake_words: Optional[list] = None) -> StreamASRSession:
        """打开 WebSocket 流式 ASR session(需手动 start / push / events / close)"""
        token = self.headers["Authorization"].split(" ", 1)[1]
        return StreamASRSession(token, wake_words=wake_words)

    def stream_asr(
        self,
        wake_words: Optional[list] = None,
        wav_path: Optional[str] = None,
        timeout: float = 30.0,
    ) -> Iterator[dict]:
        """WebSocket 流式识别 — sync generator

        用法:
            for event in c.stream_asr(wake_words=['hey oi'], wav_path='test.wav'):
                if event['type'] == 'final':
                    print(event['text'])

        wav_path 可选;若提供则自动分块推送后 stop。
        """
        session = self.open_stream_asr(wake_words=wake_words)
        session.start(timeout=timeout)
        try:
            if wav_path:
                session.push_wav_file(wav_path)
                session.stop()
            for event in session.events(timeout=timeout):
                yield event
        finally:
            session.close()

    def close(self):
        self.client.close()


# 便捷函数
def quick_speak(text: str, presence: str = "voice") -> dict:
    """一行调通:切 presence + 说一句"""
    c = VoiceInputClient()
    try:
        c.set_presence(presence)
        return c.speak(text)
    finally:
        c.close()


def quick_inject(text: str) -> dict:
    """一行调通:直接 inject 到当前焦点窗口"""
    c = VoiceInputClient()
    try:
        return c.inject_text(text)
    finally:
        c.close()