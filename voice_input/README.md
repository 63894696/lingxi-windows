# voice_input 基础版(ADR-001)— v1.0 release

> **本地语音输入工具 — 完全本地,零外发**
> **版本**:v1.0(2026-07-02 ship)
> **状态**:ADR-001 基础版完整收尾,可用

## 这是什么

**本地语音输入工具** — 给 OI agent + 真人用的语音 ASR/TTS 接口。

- **三进程架构**(ADR-001 D-1):ASR daemon (port 8731) + TTS daemon (port 8732) + Orchestrator (port 8730)
- **完全本地,零外发**:SenseVoice Small 本地 ASR + Windows SAPI 本地 TTS + 不调任何云
- **presence_mode**:`Ctrl+Shift+P` 切 `voice` / `text` / `auto`,默认 `voice` + UI 提示

## v1.0 实测数据(2026-07-02 20:11)

| 维度 | 实测结果 |
|---|---|
| mic 端到端 ASR | ✅ USB 麦克风 5s 录音 → SenseVoice 本地识别 → 返回"我现在开始说话人"(准确) |
| 真中文 wav ASR | ✅ xiaoxiao 朗诵 wav → 完整 5 句关雎(关关雎鸠在河之洲窈窕淑女君子好逑参差荇菜左右流之窈窕淑女)|
| TTS(SAPI) | ✅ 13 字符中文 4.6s 播完 |
| 文本注入 | ✅ 剪贴板 + Ctrl+V + IME 自动切英文 |
| 断网验证 | ✅ 6/6 PASSED(verify_offline.py)|
| 性能 | 本地 0.4-0.7s/10s wav(vs SILICONFLOW API 1-2s)|
| pytest | ✅ 4/4 全过 |

## v1.0 vs v0.4 增量

| 增量 | 用途 |
|---|---|
| ASR daemon `/record_and_transcribe` 同步路由 | OI 一行 mic 录音 + VAD + 转录(10s 内返回 text)|
| oi_client `record_and_transcribe()` 一键方法 | OI 在 chat 模式直接用 |
| silero-vad chunk size 修复(512 samples) | VAD 准确性 |

## 快速开始

```bash
# 1) 装
cd C:/Users/Administrator/voice_input
pip install -e .

# 2) 启动 3 daemon(各开一个终端 或 后台)
python scripts/start_all.py
# → 3 daemon 后台跑,日志写 ~/.voice_input/logs/

# 3) 测
python scripts/verify_offline.py
# → 6/6 pass

# 4) OI 端调用(在 OI chat 模式 LLM code block 里)
from clients.oi_client import VoiceInputClient
c = VoiceInputClient()

# mic 录音 + ASR(text 是用户说的内容)
text = c.record_and_transcribe(max_seconds=10)['text']
print(f'识别结果: {text}')

# OI 文本注入当前焦点窗口
c.inject_text(f'你刚才说了:{text}')

# OI 说话(走 SAPI,默认 voice mode)
c.speak(f'你说的是:{text}')

c.close()

# 5) 关 daemon
python scripts/stop_all.py
```

## REST API(ADR-001 D-5)

| Method | Path | 用途 | v1.0 状态 |
|---|---|---|---|
| `GET` | `/health` | 健康检查 | ✅ |
| `GET` | `/presence` | 当前 presence_mode | ✅ |
| `POST` | `/presence` | 切 presence_mode | ✅ |
| `POST` | `/speak` | OI 提交文本 → 路由(TTS / IM) | ✅ |
| `POST` | `/inject` | OI 文本 → 注入当前焦点窗口 | ✅ |
| `GET` | `/status` | 详细状态 | ✅ |
| `POST` | `/transcribe` | 上传 wav,识别文本 | ✅ |
| **`POST`** | **`/record_and_transcribe`** | **mic 录音 + VAD + ASR 一步完成** | ✅ v1.0 |
| `POST` | `/synthesize` | TTS 合成 + 播放 | ✅ |
| `POST` | `/synthesize_async` | 队列 TTS | ✅ |
| `POST` | `/mute` / `/unmute` | TTS 触发 ASR 静音 | ✅ |
| `POST` | `/start_listening` / `/stop_listening` | 录音起停 | ✅ |
| `POST` | `/inject`(asr_daemon) | 文本注入当前焦点窗口 | ✅ |

## OI 一键方法(voice_input/clients/oi_client.py)

```python
from clients.oi_client import VoiceInputClient, quick_inject, quick_speak

c = VoiceInputClient()
c.health()                                  # 3 daemon 总状态
c.status()                                  # 详细状态
c.set_presence("voice")                     # 切 presence_mode
text = c.record_and_transcribe(max_seconds=10)['text']  # 一键 mic + ASR
c.transcribe_wav_file("path.wav")            # 文件 ASR
c.inject_text("hello")                      # 一键 inject
c.speak("hello")                            # 一键 TTS
c.close()
```

## 隐私边界(ADR-001 §5)— 实测满足

| 维度 | 行为 | 实测验证 |
|---|---|---|
| 音频字节流上云 | ❌ 永远不上 | ✅ SenseVoice 本地 0.4-0.7s |
| TTS 调用 | ❌ Windows SAPI 本地 | ✅ SAPI 真播 13 字符中文 |
| 文本注入 | ❌ 本地剪贴板 + Ctrl+V | ✅ 65 字符注入成功 |
| 全局热键状态 | ❌ 本地 orchestrator | ✅ Ctrl+Shift+P 切 mode |
| **外发连接** | ❌ **0 个** | ✅ verify_offline 扫本机 ESTABLISHED 全 loopback |
| **ASR 文本上云** | ⚠️ **可能上**(OI 送给云 LLM) | ⚠️ OI 控制,基础版不主动送 |
| **TTS 文本上云** | ⚠️ **未来可能**(基础版锁 SAPI) | ⚠️ 用户切换 cloud TTS 时才会 |

`scripts/verify_offline.py` 是断网验证工具,启动 3 daemon + 跑 6 个 e2e 接口 + 扫本机 ESTABLISHED 非 loopback 连接。

## 目录布局

```
voice_input/
├── docs/adr/
│   └── ADR-001-local-voice-input-architecture.md  ← 完整架构说明
├── src/voice_input/
│   ├── config.py               ← 路径/端口/模型配置
│   ├── auth.py                 ← auth_token 生成/校验(256-bit)
│   ├── asr_daemon.py           ← ASR daemon (port 8731) + 14 个 REST
│   ├── tts_daemon.py           ← TTS daemon (port 8732) + SAPI + 队列
│   ├── orchestrator.py         ← Orchestrator (port 8730) + presence + mute gate
│   ├── asr/sensevoice.py       ← SenseVoice Small onnx 推理(241MB)
│   ├── asr/vad.py              ← silero-vad 6.x 包装
│   ├── tts/sapi.py             ← SAPI 占位(实装在 tts_daemon)
│   ├── audio/{capture,inject}.py ← 占位(实装在 asr_daemon)
│   └── hotkey/keyboard_global.py ← 占位(实装在 orchestrator)
├── clients/oi_client.py        ← OI 便捷客户端(5 个一键方法)
├── tests/test_basic.py         ← 4 个 pytest 单元测试
├── scripts/
│   ├── start_all.py / .bat     ← 一键启动 3 daemon
│   ├── stop_all.py / .bat      ← 一键关闭
│   ├── verify_offline.py       ← 断网 e2e 验证(6 项)
│   └── gen_auth_token.py       ← 手动生成 auth_token
├── pyproject.toml
└── README.md
```

## Git log

```
v1.0 (commit 5)  ASR daemon 加 /record_and_transcribe + OI 一键 mic + README release
v0.4 (commit 4)  OI clients 集成 + orchestrator /inject + 一键启动脚本
v0.3 (commit 3)  AudioCapture VAD + 文本注入 + verify_offline 6/6 PASSED
v0.2 (commit 2)  SenseVoice Small onnx 推理完整实装
v0.1 (commit 1)  基础版三进程骨架 + REST 接口
```

## v1.1+ 路线(下次接哪里)

| # | 任务 | 工作量 |
|---|---|---|
| 1 | ADR-002 FastLane 启动(云端优先) | 用户拍板 |
| 2 | ADR-003 GhostLine 启动(SimpleX IM) | 用户拍板 |
| 3 | 持续录音流(WebSocket /ws 流式识别) | 1 天 |
| 4 | SAPI 声音质量改进(VITS 本地神经网络 TTS) | 半天 |
| 5 | 唤醒词支持("Hey OI" 自动唤醒) | 1 天 |

## 给用户的使用提示

1. **确保闪电说已装**(SenseVoice 模型文件路径走 `%AppData%\Shandianshuo\models\sensevoice-small\` 复用)
2. **USB 麦克风设为默认输入设备**(`sounddevice.default.device[0]` 默认 1)
3. **按 Ctrl+Shift+P 切 presence_mode** — 在 voice 模式 SAPI 播,在 text 模式 IM 注入
4. **跑 `verify_offline.py` 验证隐私** — 启动 daemon 之前建议断网或开 firewall
5. **OI 在 chat 模式 code block 直接调** `c.record_and_transcribe()` — 一行 mic 录音 + ASR

## 贡献与归档

- **voice_agent/(旧云端 daemon, 935 行)** 已归档,不再演进。详见 handoff §1.3
- 3 个 ADR 仓库独立,FastLane / GhostLine 不共享 voice_input 代码(隐私审计风险)
- ADR-001 §6 触发再讨论条件见 ADR 文档

---

**🎉 voice_input 基础版 v1.0 ship 完毕!**

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>