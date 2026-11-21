# Privacy Boundary — `voice_input` 项目

> 与 ADR-001 配套的隐私边界声明。**代码层面强制 + 文档层面声明 + 用户可控开关** 三层保障。

## 0. 一句话承诺

> **音频字节流永远不上云。文本字节流可能上云——但由用户决定。**

## 1. 不上云的部分(代码层强制)

| 模块 | 数据类型 | 流向 | 强制手段 |
|---|---|---|---|
| `asr_daemon` 麦克风采集 | PCM 16kHz mono | mic → sounddevice → VAD → SenseVoice ONNX 推理 → 文本 | 代码审计 + 单元测试 + Wireshark 抓包 |
| `tts_daemon` TTS 调用 | text → audio | text → Windows SAPI(本地 COM/DLL) → 扬声器 | 代码路径锁定到 `win32com.client.Dispatch("SAPI.SpVoice")`,无其他 TTS 后端 |
| `orchestrator` 热键/静音信号 | 控制信号 | 本机 loopback HTTP | daemon bind 仅 `127.0.0.1`,代码层拒绝 `0.0.0.0` |
| 文本注入到当前焦点 | text | clipboard → Ctrl+V → OS | 不发任何网络请求 |

### 1.1 强制手段的具体实现

- **代码层**:`scripts/audit_network_calls.py` 静态扫描 src/,grep `urllib`、`requests`、`http.client`、`socket.connect`,**仅允许指向 `127.0.0.1` / `localhost`**。CI 失败 → 拒绝合并。
- **运行层**:`scripts/verify_offline.py` 启动所有 daemon,跑完整 e2e,Wireshark 抓包,断言「非 loopback 外发请求数 == 0」。
- **防火墙层**(可选):用户可在系统防火墙层加规则 `block voice_input.exe except 127.0.0.1`。

## 2. 可能上云的部分(由用户决定)

| 模块 | 数据类型 | 流向 | 用户控制 |
|---|---|---|---|
| ASR 输出文本(text) | 字符串 | ASR daemon → orchestrator → OI → OI 配的 LLM API | OI 进程配什么 LLM 完全由用户控制(本地 Ollama / 云 Qwen / 云 GPT 等) |
| OI 提交给 TTS 的文本(text) | 字符串 | OI → orchestrator → TTS daemon → SAPI(本地) | POC 锁 SAPI,**未来替换云 TTS 需改 ADR 并用户确认** |
| 用户配置文件 | JSON | 本地 `~/.voice_input/config.json` | 永不上云,代码无上传路径 |

### 2.1 用户可审计性

- 日志 `~/.voice_input/logs/<date>.log` 记录所有 daemon 间 IPC(loopback HTTP 请求 + 响应体摘要),**不记录音频或文本完整内容**(避免日志变成隐私泄漏面)。
- 用户可读日志,但**日志只在本机,无外发路径**。

## 3. 用户可关闭 / 启用的开关

| 开关 | 默认 | 描述 |
|---|---|---|
| `daemons.run_asr_on_startup` | True | 系统启动时 ASR daemon 自动 run |
| `daemons.run_tts_on_startup` | True | 系统启动时 TTS daemon 自动 run |
| `daemons.run_orchestrator_on_startup` | True | 系统启动时 Orchestrator 自动 run |
| `presence.default_mode` | `voice` | Orchestrator 启动时的 `presence_mode` |
| `asr.use_local_only` | True | True = 强制本地 ASR,False = 允许加载云 ASR 后端(未实现) |
| `tts.engine` | `sapi` | `sapi`(本地)/ `cloud`(未实现)/ `local_nn`(未实现) |
| `logging.level` | `INFO` | `DEBUG` 会记录更详细的事件 |
| `logging.log_request_bodies` | False | True 会记录文本内容(隐私泄漏面,**默认关**) |

## 4. 不收集的内容(显式声明)

- **不采集**:摄像头画面、键盘按键、屏幕内容、文件路径、应用程序列表、操作系统版本号、用户输入习惯。
- **不上报**:任何崩溃报告、任何使用统计。
- **不联网**:所有 daemon 启动后只监听 `127.0.0.1`,**无 telemetry、无 license check、无远程配置**。

## 5. 与闪电说的对比

| 维度 | 闪电说 | voice_input |
|---|---|---|
| ASR 默认路径 | 内建本地(SenseVoice)+ 云端备选 | **强制本地** SenseVoice,无云端 |
| 「帮我说」功能 | 走云端大模型(必) | **POC 不实现**(OI 替代) |
| 音频字节流 | 默认本地 ASR 时不上云 | **永远不上云** |
| ASR 文本 | 默认走云端大模型纠错 | **走 OI agent**,用户决定 OI 配什么 LLM |
| 摄像头 | 无 | **永不接入** |
| 数据收集声明 | 文档未给(我们已记入 memory) | **本文档 + ADR 强制审计** |

## 6. 未来替换云端组件时的隐私影响(预声明)

如果未来用户决定:

- **替换 TTS 为云端** → 隐私承诺变化:OI 的回复文本会上云。需 ADR + 用户 explicit consent。
- **替换 ASR 为云端** → 隐私承诺变化:音频字节流会上云(违背本文档 §1)。
- **引入摄像头人脸检测** → 隐私承诺变化:画面帧会上摄像头检测(本地),但仍违背「永不接入摄像头」原则,需 explicit consent。

**任何替换都先改 ADR + privacy.md,不改不能改代码**。