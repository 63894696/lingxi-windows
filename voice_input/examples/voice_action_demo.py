"""Voice Action Demo v1.3(2026-07-02 21:15 ship)

核心区别 v1.2:不是被动 inject,而是**主动执行指令**
- v1.2 (输入法):用户说"hey oi 在记事本写 hello" → 只 inject 文字(用户必须自己开 notepad)
- v1.3 (agent):用户说"打开记事本在里面打字 hello" → agent 解析 + 主动开 notepad + 打字

v1.3 流程:
  1. mic 录音(用户说完整指令,如"打开记事本在里面打字 hello")
  2. SenseVoice 识别
  3. 简单规则解析(暂不调 LLM,关键词匹配 → [ACTION:json])
  4. 解析出 actions 列表:[{"action":"open","target":"notepad"}, {"action":"type","text":"hello"}]
  5. 逐步执行:
     - open: subprocess.Popen("notepad.exe")
     - 等待 1.5s 让 notepad 获得焦点
     - type: 调 desktop.type_text("hello") → pyautogui.typewrite
  6. 真在 notepad 看到 hello

v1.4+ 计划:
- 解析层从关键词匹配升级到 LLM 决策(STEPFUN step-3.7-flash,免费额度多)
- 加更多 action 类型(click / hotkey / find_window / screenshot)
- voice_input /execute 路由(类似 voice_agent Layer 5)

跑法:
  python examples/voice_action_demo.py
"""
from __future__ import annotations
import os
import sys
import re
import time
import subprocess
import json
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
os.chdir(PROJECT_ROOT)
sys.path.insert(0, str(PROJECT_ROOT / "src"))
sys.path.insert(0, str(PROJECT_ROOT / "clients"))
sys.path.insert(0, "C:/Users/Administrator/oi_enhancements")

from voice_input.asr_daemon import detect_wake_word
from clients.oi_client import VoiceInputClient


# ============================================================
# 简单指令解析(关键词 + 正则)— 未来升级 LLM
# ============================================================
WAKE_KEYWORDS = ["hey oi", "你好", "嗨", "oi", "hello oi"]


def parse_command(text: str) -> list:
    """v1.4 ship:LLM 决策优先 + 正则 fallback"""
    try:
        return parse_command_llm(text)
    except Exception as e:
        log_local(f"  LLM 解析失败:{e},fallback 正则版")
        return parse_command_keywords(text)


# ============================================================
# LLM 决策层(2026-07-02 v1.4 ship)
# ============================================================
import urllib.request as _ur
import urllib.error as _ue
import winreg as _wr


def _read_env(name: str) -> str:
    """从 Windows 注册表读 env var"""
    try:
        reg = _wr.OpenKey(_wr.HKEY_CURRENT_USER, "Environment")
        val, _ = _wr.QueryValueEx(reg, name)
        _wr.CloseKey(reg)
        return val
    except FileNotFoundError:
        return ""


_log_local = lambda msg: None


def _set_local_logger(fn):
    global _log_local
    _log_local = fn


def parse_command_llm(text: str, model: str = "step-3.7-flash") -> list:
    """用 STEPFUN LLM 解析自然语言 → JSON actions 列表(2026-07-02 v1.4 ship)

    优势:不依赖正则,处理变体 / 复杂指令 / 多 action 串联
    """
    api_key = _read_env("STEPFUN_API_KEY")
    if not api_key:
        raise RuntimeError("STEPFUN_API_KEY not in env")

    system_prompt = """你是 voice_input 指令解析器。把用户的中文/英文语音指令解析成 JSON 动作列表。

可用的 action 类型:
1. open:打开应用,target 可以是 notepad / chrome / mspaint / calc / cmd
2. type:在当前焦点窗口输入文字 {"action":"type","text":"..."}
3. click:鼠标点击 {"action":"click","x":N,"y":N}
4. key:按键 {"action":"key","key":"enter"}
5. close:关闭窗口 {"action":"close"}
6. speak:语音播报(TTS){"action":"speak","text":"..."}

输出格式:严格 JSON 数组,只返回 JSON。例:
[{"action":"open","target":"notepad"},{"action":"type","text":"hello"},{"action":"speak","text":"已完成"}]"""

    body = _ur.Request.__class__  # 占位
    import json as _json
    body_str = _json.dumps({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": text},
        ],
        "max_tokens": 1000,
        "temperature": 0.1,
    }, ensure_ascii=False).encode("utf-8")

    req = _ur.Request(
        "https://api.stepfun.com/v1/chat/completions",
        data=body_str,
        headers={"Authorization": f"Bearer {api_key}", "Content-Type": "application/json"},
        method="POST",
    )
    try:
        with _ur.urlopen(req, timeout=30) as r:
            payload = _json.loads(r.read().decode("utf-8"))
    except _ue.HTTPError as e:
        body_e = e.read().decode("utf-8", errors="ignore")[:200]
        raise RuntimeError(f"STEPFUN HTTP {e.code}: {body_e}")
    except Exception as e:
        raise RuntimeError(f"STEPFUN call ERR: {e}")

    msg = payload["choices"][0]["message"]
    content = msg.get("content") or msg.get("reasoning_content") or ""
    content = content.strip()
    # 去 markdown 包装
    if content.startswith("```"):
        lines = content.split("
")
        content = "
".join(l for l in lines if not l.strip().startswith("```"))
    try:
        actions = _json.loads(content)
    except _json.JSONDecodeError as e:
        raise RuntimeError(f"LLM 输出非 JSON:{content[:200]}")
    if not isinstance(actions, list):
        raise RuntimeError(f"LLM 输出不是 list")
    return actions


def parse_command_keywords(text: str) -> list:
    """v1.3.2 关键词+正则版(2026-07-02 ship)

    返回:[{"action": "open", "target": "notepad"}, {"action": "type", "text": "hello"}, ...]

    支持的 pattern:
      - "打开[notepad/记事本/...]"   → action=open
      - "在里面打字[xxx]"           → action=type
      - "点击[x,y]"                  → action=click
      - "关闭[xxx]"                  → action=close
      - "[文本] [enter/回车]"        → action=key,key=enter
    """
    actions = []
    text_clean = text.strip()

    # 1. 剥掉 wake word
    for w in WAKE_KEYWORDS:
        if w.lower() in text_clean.lower():
            text_clean = re.sub(w, "", text_clean, flags=re.IGNORECASE).strip()
            break

    # 2. 检测 open 动作(关键词:"打开" 后跟 app 名)— v1.3.1 修正则到下一个分隔词
    open_match = re.search(
        r"打开\s*(\S+?)(?=\s*(?:在|然后|打字|输入|写|里面|$))",
        text_clean,
    )
    if open_match:
        target = open_match.group(1).strip()
        # 中文 → 英文映射
        target_map = {
            "记事本": "notepad",
            "浏览器": "chrome",
            "画图": "mspaint",
            "计算器": "calc",
            "终端": "cmd",
        }
        target = target_map.get(target, target)
        actions.append({"action": "open", "target": target})
        # v1.3.1:删掉 open 段,避免 type 正则重复吃
        text_clean = text_clean.replace(open_match.group(0), "").strip()

    # 3. 检测 type 动作 — v1.3.1 改:取 open 之后剩余文本(避免吃整段)
    type_match = re.search(
        r"(?:打字|输入|写)(?:一下|里头|里面)?\s*(.+?)(?:[。.!?]|$)",
        text_clean,
    )
    if type_match:
        text_to_type = type_match.group(1).strip()
        for noise in ["在里面", "那里", "然后", "一下", "里头"]:
            text_to_type = text_to_type.replace(noise, "").strip()
        if text_to_type:
            actions.append({"action": "type", "text": text_to_type})

    # 4. 检测 click 动作(关键词:"点击 100,200")
    click_match = re.search(r"点击\s*(\d+)\s*[, ]\s*(\d+)", text_clean)
    if click_match:
        x, y = int(click_match.group(1)), int(click_match.group(2))
        actions.append({"action": "click", "x": x, "y": y})

    # 4. v1.3.2 新增 speak 动作 — 关键词:speak/朗读/语音/tts/念/回复/回答/说
    speak_match = re.search(
        r"(?:speak|tts|朗读|语音|念(?:出)?|回[复答]?|说)\s*(?:一下|给我|跟我|给)?\s*(.+?)(?:[。.!?]|$)",
        text_clean,
    )
    if speak_match:
        text_to_speak = speak_match.group(1).strip()
        # 清前置(用 t t s 跟 我 说 等 变体)
        text_to_speak = re.sub(r"^用\s*t\s*t\s*s\s*", "", text_to_speak, flags=re.IGNORECASE).strip()
        text_to_speak = re.sub(r"^用\s*tts\s*", "", text_to_speak, flags=re.IGNORECASE).strip()
        text_to_speak = re.sub(r"^(?:我[说给]|跟[我说]|给我|跟我)\s*", "", text_to_speak).strip()
        text_to_speak = re.sub(r"^回复\s*", "", text_to_speak).strip()
        # 如果开头就"说/念/朗读"等单字,剥掉
        text_to_speak = re.sub(r"^[说念朗读]\s*", "", text_to_speak).strip()
        for sep in [",", "，", "然后", "再"]:
            if sep in text_to_speak:
                text_to_speak = text_to_speak.split(sep)[0].strip()
        if text_to_speak and 0 < len(text_to_speak) < 200:
            actions.append({"action": "speak", "text": text_to_speak})

    # 5. 检测 key 动作(enter / 回车 / escape)
    if re.search(r"(enter|回车|确定)", text_clean, re.IGNORECASE):
        actions.append({"action": "key", "key": "enter"})

    # 6. 检测 close 动作(关键词:"关闭")
    if "关闭" in text_clean or "close" in text_clean.lower():
        actions.append({"action": "close"})

    return actions


# ============================================================
# Action 执行(2026-07-02 v1.3 ship)
# ============================================================
def execute_action(action: dict) -> str:
    """执行一个 action,返回执行结果描述(2026-07-02 v1.3.3 修 UnboundLocalError)"""
    import subprocess as sp  # 防御性 import(避免上层 import 冲突)
    import platform as _pl
    action_type = action.get("action")
    log = []

    if action_type == "open":
        target = action.get("target", "")
        log.append(f"  打开 {target}")
        if target == "notepad":
            sp.Popen("notepad.exe", shell=False)
            log.append(f"  ✓ notepad.exe 启动,等 1.5s 焦点")
            time.sleep(1.5)
        elif target in ("chrome", "msedge", "firefox"):
            sp.Popen(f"start {target}", shell=True)
            time.sleep(2)
        elif target in ("mspaint", "calc"):
            sp.Popen(f"start {target}", shell=True)
            time.sleep(1.5)
        else:
            # 尝试 sp.Popen 通用启动
            sp.Popen(f"start {target}", shell=True)
            time.sleep(2)

    elif action_type == "type":
        text = action.get("text", "")
        if not text:
            return f"  ✗ type 缺 text 参数"
        log.append(f"  输入 {text!r}")
        try:
            import pyautogui
            pyautogui.FAILSAFE = False
            # 先确保焦点在前(用 win32 切英文 IME,避免中文 IME 模式粘出乱码)
            try:
                import ctypes
                import ctypes.wintypes as wt
                user32 = ctypes.WinDLL("user32", use_last_error=True)
                imm32 = ctypes.WinDLL("imm32", use_last_error=True)
                hwnd = user32.GetForegroundWindow()
                if hwnd:
                    himc = imm32.ImmGetContext(hwnd)
                    if himc:
                        IME_CMODE_NATIVE = 0x0001
                        conv = wt.DWORD(0)
                        sent = wt.DWORD(0)
                        if imm32.ImmGetConversionStatus(himc, ctypes.byref(conv), ctypes.byref(sent)):
                            new_conv = conv.value & ~IME_CMODE_NATIVE
                            imm32.ImmSetConversionStatus(himc, new_conv, sent.value)
                        imm32.ImmReleaseContext(hwnd, himc)
            except Exception:
                pass
            pyautogui.typewrite(text, interval=0.02)
            log.append(f"  ✓ 输入完成")
        except Exception as e:
            log.append(f"  ✗ 输入失败:{e}")

    elif action_type == "click":
        x, y = action.get("x", 0), action.get("y", 0)
        log.append(f"  点击 ({x}, {y})")
        try:
            import pyautogui
            pyautogui.click(x, y)
        except Exception as e:
            log.append(f"  ✗ 点击失败:{e}")

    elif action_type == "key":
        key = action.get("key", "")
        log.append(f"  按键 {key}")
        try:
            import pyautogui
            pyautogui.press(key)
        except Exception as e:
            log.append(f"  ✗ 按键失败:{e}")

    elif action_type == "close":
        log.append(f"  关闭当前窗口")
        try:
            import pyautogui
            pyautogui.hotkey("alt", "f4")
        except Exception as e:
            log.append(f"  ✗ 关闭失败:{e}")

    elif action_type == "speak":
        text = action.get("text", "")
        if not text:
            return f"  ✗ speak 缺 text 参数"
        log.append(f"  语音播报: {text!r}")
        try:
            # 优先用 voice_input.audio(支持 BAILIAN → EDGE 三层 fallback)
            from voice_input.audio import tts as audio_tts
            wav_out = Path(os.environ.get("TEMP", "/tmp")) / f"voice_input_speak_{int(time.time()*1000)}.wav"
            ok = audio_tts(text, str(wav_out), voice="sambert-zhichu-v1", platform="BAILIAN")
            if ok:
                log.append(f"  ✓ TTS 成功: {wav_out} ({wav_out.stat().st_size}b)")
                # 播放(Windows 用 winsound / macOS afplay / Linux aplay)
                try:
                    import platform, subprocess
                    sys_plat = platform.system()
                    if sys_plat == "Windows":
                        import winsound
                        winsound.PlaySound(str(wav_out), winsound.SND_FILENAME)
                        log.append(f"  ✓ 播放完成")
                    elif sys_plat == "Darwin":
                        sp.Popen(["afplay", str(wav_out)])
                    else:
                        sp.Popen(["aplay", str(wav_out)])
                except Exception as e:
                    log.append(f"  ⚠ 播放失败(已生成 wav):{e}")
            else:
                log.append(f"  ✗ TTS 失败(可能 KEY 失效),用 print 替代")
                print(f"  [TTS 替代] {text}")
        except ImportError as e:
            log.append(f"  ✗ voice_input.audio 模块不可用:{e}")
            print(f"  [TTS 替代] {text}")
        except Exception as e:
            log.append(f"  ✗ TTS ERR:{type(e).__name__}: {e}")
            print(f"  [TTS 替代] {text}")

    return "\n".join(log)


# ============================================================
# 主流程
# ============================================================
def run_demo():
    print("=" * 60)
    print("voice_input Voice Action Demo (v1.3 ship 2026-07-02 21:15)")
    print(f"  项目根: {PROJECT_ROOT}")
    print("=" * 60)
    print()
    print("  本 demo 把语音指令 → [ACTION:json] → 实际执行")
    print("  默认指令:'打开记事本在里面打字 hello'(完整跑通)")
    print()

    c = VoiceInputClient()
    try:
        health = c.health()
        print(f"  [/health] status={health.get('status')}")
    except Exception as e:
        print(f"  ✗ /health 失败:{e}")
        return

    print()
    print("  按 Enter 开始 mic 录音(最长 10s,说话后 VAD 自动停)...")
    try:
        input()
    except EOFError:
        pass

    print()
    print("[1] mic 录音(说'打开记事本在里面打字 hello')...")
    r = c.record_and_transcribe(max_seconds=10, input_device=1)
    text = r.get("text", "")
    print(f"  识别: {text!r}")

    if not text:
        print("  ✗ 识别为空,退出")
        return

    print()
    print("[2] 解析指令 → [ACTION:json]...")
    actions = parse_command(text)
    print(f"  解析结果:{len(actions)} 个 action")
    for i, a in enumerate(actions):
        print(f"    [{i+1}] {a}")

    if not actions:
        print("  ⚠ 未解析出任何 action(可能是描述性的话,如'今天天气好')")
        return

    print()
    print(f"[3] 执行 {len(actions)} 个 action...")
    for i, a in enumerate(actions):
        print(f"  [{i+1}/{len(actions)}] action: {a}")
        result = execute_action(a)
        print(result)

    print()
    print("=" * 60)
    print(f"  ✓ Demo 完成!")
    print(f"  识别:   {text!r}")
    print(f"  解析:   {len(actions)} 个 action")
    print(f"  → 记事本应已打开 + 含 'hello' 文字")
    print("=" * 60)


if __name__ == "__main__":
    run_demo()