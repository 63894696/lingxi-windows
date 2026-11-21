# -*- coding: utf-8 -*-
"""右 Alt 全局听写工具(2026-08-17)

闪电说平替 + 智能体语音输入:按右 Alt 激活 → 讲话 → 再按右 Alt → 语音转文字 → 光标位置输出。
复用闪电说 SenseVoice Small 模型(本地推理,零外发)。

用法:
  python dictation_tool.py            # 启动(系统托盘图标)
  右 Alt                               # 开始录音
  讲话...
  右 Alt                               # 停止并识别 → 文字打到光标处

依赖: onnxruntime, pynput, sounddevice, numpy, pystray, Pillow
模型: 自动复用 %APPDATA%\\Shandianshuo\\models\\sensevoice-small\\
"""
from __future__ import annotations
import io
import json
import sys
import threading
import time
import wave
from pathlib import Path

import numpy as np
import onnxruntime as ort
import sounddevice as sd

# ---------- SenseVoice 推理(复用 voice_input/sensevoice.py 逻辑,自包含) ----------
SAMPLE_RATE = 16000
N_MELS = 80
FRAME_LENGTH_MS = 25
FRAME_SHIFT_MS = 10
LFR_M = 7
LFR_N = 6
N_FFT = 512
PREEMPH_COEF = 0.97

MODEL_DIR = (
    Path.home() / "AppData" / "Roaming" / "Shandianshuo" / "models" / "sensevoice-small"
)


def _hamming(L):
    return 0.54 - 0.46 * np.cos(2 * np.pi * np.arange(L) / (L - 1))


def _preemphasis(x, coef=PREEMPH_COEF):
    if coef <= 0:
        return x
    return np.append(x[0], x[1:] - coef * x[:-1])


def _mel_fb(n_mels, n_fft, sr):
    def h2m(f):
        return 2595.0 * np.log10(1.0 + f / 700.0)

    def m2h(m):
        return 700.0 * (10.0 ** (m / 2595.0) - 1.0)

    mel_pts = np.linspace(h2m(0.0), h2m(sr / 2.0), n_mels + 2)
    hz_pts = m2h(mel_pts)
    bin_freq = np.linspace(0, sr / 2, n_fft // 2 + 1)
    fb = np.zeros((n_mels, n_fft // 2 + 1), dtype=np.float32)
    for i in range(n_mels):
        left, center, right = hz_pts[i], hz_pts[i + 1], hz_pts[i + 2]
        for j in range(len(bin_freq)):
            f = bin_freq[j]
            if f < left or f > right:
                continue
            if f <= center:
                fb[i, j] = (f - left) / (center - left)
            else:
                fb[i, j] = (right - f) / (right - center)
    return fb


def extract_features(audio, sr=SAMPLE_RATE):
    audio = _preemphasis(audio, PREEMPH_COEF)
    flen = int(FRAME_LENGTH_MS * sr / 1000)
    fshift = int(FRAME_SHIFT_MS * sr / 1000)
    win = _hamming(flen)
    fb = _mel_fb(N_MELS, N_FFT, sr)
    n_frames = max(1, (len(audio) - flen) // fshift + 1)
    if len(audio) < flen:
        audio = np.pad(audio, (0, flen - len(audio)))
    frames = np.lib.stride_tricks.as_strided(
        audio,
        shape=(n_frames, flen),
        strides=(audio.strides[0] * fshift, audio.strides[0]),
    ).copy()
    frames = frames * win
    spec = np.fft.rfft(frames, n=N_FFT, axis=1)
    power = np.abs(spec) ** 2
    mel = power @ fb.T
    mel_log = np.log10(np.maximum(mel, 1e-10))
    mean = mel_log.mean(axis=0, keepdims=True)
    std = mel_log.std(axis=0, keepdims=True) + 1e-5
    mel_log = (mel_log - mean) / std
    if n_frames < LFR_M:
        mel_log = np.concatenate([mel_log, np.zeros((LFR_M - n_frames, N_MELS), dtype=np.float32)])
        n_frames = LFR_M
    n_lfr = (n_frames - LFR_M) // LFR_N + 1
    out = np.zeros((n_lfr, N_MELS * LFR_M), dtype=np.float32)
    for i in range(n_lfr):
        out[i] = mel_log[i * LFR_N : i * LFR_N + LFR_M].flatten()
    return out


def greedy_decode(logits, id2tok, blank_id=0):
    preds = logits.argmax(axis=-1)
    result = []
    prev = -1
    for p in preds:
        if p != prev and p != blank_id:
            result.append(int(p))
        prev = p
    text = ""
    for idx in result:
        if idx >= len(id2tok):
            continue
        tok = id2tok[idx]
        if tok.startswith("<|") and tok.endswith("|>"):
            continue
        if tok == "▁":
            text += " "
        else:
            text += tok.replace("▁", " ")
    return text.strip()


class SenseVoice:
    def __init__(self):
        self.session = None
        self.id2tok = None

    def load(self):
        if not (MODEL_DIR / "model.onnx").exists() or not (MODEL_DIR / "tokens.json").exists():
            return False, f"模型未找到: {MODEL_DIR}"
        try:
            self.session = ort.InferenceSession(
                str(MODEL_DIR / "model.onnx"), providers=["CPUExecutionProvider"]
            )
            toks = json.loads((MODEL_DIR / "tokens.json").read_text(encoding="utf-8"))
            self.id2tok = {i: t for i, t in enumerate(toks)}
            return True, f"模型已加载 (vocab {len(self.id2tok)})"
        except Exception as e:
            return False, f"模型加载失败: {e}"

    def transcribe(self, pcm_float, sr=SAMPLE_RATE, language=0, textnorm=14):
        # language: 0=auto(自动检测),3=zh,4=en(原代码误用 1)
        # textnorm: 14=withitn(带标点+数字规整),15=woitn(不带)
        #   —— 2026-08-17 实测无标点根因:原代码 textnorm=2 是错误值,
        #   SenseVoice 正确值是 14(with-itn)才有标点,闪电说正是用它。
        if self.session is None:
            return ""
        feats = extract_features(pcm_float, sr)
        feed = {
            "speech": feats[np.newaxis, :, :].astype(np.float32),
            "speech_lengths": np.array([feats.shape[0]], dtype=np.int32),
            "language": np.array([language], dtype=np.int32),
            "textnorm": np.array([textnorm], dtype=np.int32),
        }
        logits = self.session.run(None, feed)[0][0]
        return greedy_decode(logits, self.id2tok)


# ---------- 录音 ----------
# 用 sounddevice callback 模式(非阻塞,官方推荐),不用阻塞 read()——
# 阻塞 read 在另一线程 close 流时会崩(2026-08-17 第二次按 Alt 闪退根因)。
class Recorder:
    def __init__(self):
        self._frames = []
        self._stream = None
        self.recording = False
        self._lock = threading.Lock()

    def _callback(self, indata, frames, time_info, status):
        # sounddevice 回调线程:只追加帧,不做重活,绝不抛异常
        try:
            if self.recording:
                self._frames.append(indata.copy())
        except Exception:
            pass

    def start(self):
        with self._lock:
            self._stop_locked()  # 先确保旧流关闭(防第二次按时资源冲突)
            self._frames = []
            self.recording = True
            try:
                self._stream = sd.InputStream(
                    samplerate=SAMPLE_RATE,
                    channels=1,
                    dtype="int16",
                    blocksize=1024,
                    callback=self._callback,
                )
                self._stream.start()
            except Exception:
                self.recording = False
                self._stream = None

    def _stop_locked(self):
        self.recording = False
        if self._stream:
            try:
                self._stream.stop()
            except Exception:
                pass
            try:
                self._stream.close()
            except Exception:
                pass
            self._stream = None

    def stop(self):
        with self._lock:
            self._stop_locked()
            if not self._frames:
                return np.array([], dtype=np.float32)
            try:
                pcm = np.concatenate(self._frames, axis=0).flatten().astype(np.float32) / 32768.0
            except Exception:
                pcm = np.array([], dtype=np.float32)
            return pcm


# ---------- 光标打字 ----------
# 关键(2026-08-17 用户实测文字打到程序自己而非记事本):
# 按右 Alt 时焦点可能已离开目标窗口(记事本)。必须在激活时记住前台窗口,
# 识别完先把焦点切回那个窗口,再输入文字 —— 闪电说的做法。
#
# 闪退修复(2026-08-17 第二次实测):ctypes SendInput 的复杂结构在部分环境
# 会导致进程级硬崩溃(非 Python 异常,直接闪退无日志)。改用简单可靠的
# 剪贴板 + Ctrl+V(pyautogui,原 voice_input asr_daemon 已验证)。
def _get_foreground_hwnd():
    try:
        import ctypes
        return ctypes.windll.user32.GetForegroundWindow()
    except Exception:
        return 0


def _set_foreground_hwnd(hwnd):
    if not hwnd:
        return
    try:
        import ctypes
        user32 = ctypes.windll.user32
        # 简单版:SetForegroundWindow + ShowWindow(复杂 AttachThreadInput 易崩)
        user32.ShowWindow(hwnd, 9)  # SW_RESTORE
        user32.SetForegroundWindow(hwnd)
    except Exception:
        pass


def type_text(text):
    if not text:
        return
    # 剪贴板 + Ctrl+V:最兼容,中文/英文/符号都行,不会进程崩溃。
    # 用 pyperclip 设剪贴板(比 powershell 快),pyautogui 发 Ctrl+V。
    try:
        import pyperclip
        pyperclip.copy(text)
    except Exception:
        # pyperclip 没装 → 用 tkinter 设剪贴板(标准库)
        try:
            import tkinter as tk
            r = tk.Tk()
            r.withdraw()
            r.clipboard_clear()
            r.clipboard_append(text)
            r.update()
            r.destroy()
        except Exception:
            pass
    try:
        import pyautogui
        time.sleep(0.05)  # 等剪贴板/焦点就绪
        pyautogui.hotkey("ctrl", "v")
    except Exception:
        pass


# ---------- 主逻辑 ----------
class DictationApp:
    def __init__(self):
        self.asr = SenseVoice()
        self.recorder = Recorder()
        self.status = "init"  # init / ready / recording / transcribing / error
        self.status_msg = "初始化中…"
        self._lock = threading.Lock()
        self._alt_pressed = False
        self._tray = None

    def set_status(self, s, msg=""):
        with self._lock:
            self.status = s
            self.status_msg = msg
        self._update_tray()

    def _update_tray(self):
        if self._tray:
            try:
                self._tray.title = f"听写: {self.status_msg}"
                self._tray.icon = self._make_icon()
            except Exception:
                pass

    def _make_icon(self):
        from PIL import Image, ImageDraw

        img = Image.new("RGB", (64, 64), "#1a1a2e")
        d = ImageDraw.Draw(img)
        colors = {
            "ready": "#3fbf7f",
            "recording": "#e05252",
            "transcribing": "#e8b23c",
            "error": "#888888",
            "init": "#5a636f",
        }
        c = colors.get(self.status, "#5a636f")
        d.ellipse([14, 8, 50, 44], fill=c)  # 麦克风头
        d.rectangle([28, 40, 36, 54], fill=c)  # 杆
        d.rectangle([20, 54, 44, 58], fill=c)  # 底座
        return img

    def on_alt_press(self):
        # 全程异常防护:任何一步崩都不能让进程闪退(2026-08-17 第二次按闪退)
        try:
            if self.status == "recording":
                # 第二次按 → 停止并识别
                self.stop_and_transcribe()
            elif self.status == "ready":
                # 第一次按 → 开始录音。先记住当前前台窗口(记事本等),
                # 识别完要切回去再打字(2026-08-17 实测文字打到程序自己的修复)。
                self._target_hwnd = _get_foreground_hwnd()
                self.recorder.start()
                self.set_status("recording", "录音中… 再按右 Alt 停止")
        except Exception as e:
            try:
                self.set_status("ready", f"就绪(出错已恢复)")
                print(f"[WARN] on_alt_press 异常已捕获: {e}", file=sys.stderr)
            except Exception:
                pass

    def stop_and_transcribe(self):
        try:
            self.set_status("transcribing", "识别中…")
            pcm = self.recorder.stop()
            if len(pcm) < SAMPLE_RATE * 0.3:  # <0.3s 太短
                self.set_status("ready", "就绪(录音太短)")
                return
            text = self.asr.transcribe(pcm)
            if text:
                # 先把焦点切回激活时的目标窗口(记事本),再打字 —— 否则文字进程序自己
                if getattr(self, "_target_hwnd", 0):
                    _set_foreground_hwnd(self._target_hwnd)
                    time.sleep(0.05)  # 等焦点切换生效
                type_text(text)
                self.set_status("ready", f"✓ {text[:20]}")
            else:
                self.set_status("ready", "就绪(未识别到内容)")
        except Exception as e:
            try:
                self.set_status("ready", "就绪(识别出错已恢复)")
                print(f"[WARN] stop_and_transcribe 异常已捕获: {e}", file=sys.stderr)
            except Exception:
                pass

    def run(self):
        ok, msg = self.asr.load()
        if not ok:
            self.set_status("error", msg)
            print(f"[ERROR] {msg}", file=sys.stderr)
        else:
            self.set_status("ready", "就绪 — 按右 Alt 开始")
            print(f"[OK] {msg}")

        # 右 Alt 监听(用 keyboard 库,不用 pynput —— 2026-08-17 实测:
        # pynput 的 Listener/GlobalHotKeys 在某些 Windows 终端需管理员权限且静默失败,
        # 正是 v1.2.4 文档记录的坑。keyboard 库无需管理员、全局热键稳定。)
        #
        # suppress=True(2026-08-17 记事本实测):右 Alt(AltGr)在记事本等应用里是
        # 菜单激活键,suppress=False 会让 Alt 键同时传给前台应用 → 触发菜单栏焦点,
        # 覆盖我们的热键。suppress=True 拦截按键,只触发我们的回调,不传给前台,
        # 这就是闪电说在记事本里不触发菜单的做法。
        #
        # 精确区分左右 Alt(2026-08-17 左 Alt+Tab 被误锁修复):
        # keyboard 库的 "right alt" 底层映射 VK_RMENU(右 Alt),左 Alt 是 VK_LMENU,
        # 两者是不同虚拟键。只注册 "right alt" → 左 Alt(VK_LMENU)完全不注册,
        # Alt+Tab 窗口切换不受影响(闪电说同样区分左右修饰键)。
        try:
            import keyboard as kb_lib

            def _on_right_alt(e):
                if e.event_type == "down" and not self._alt_pressed:
                    self._alt_pressed = True
                    threading.Thread(target=self.on_alt_press, daemon=True).start()
                elif e.event_type == "up":
                    self._alt_pressed = False

            kb_lib.on_press_key("right alt", _on_right_alt, suppress=True)
            kb_lib.on_release_key("right alt", _on_right_alt, suppress=True)
            print("[OK] 右 Alt 热键已注册 (keyboard 库, suppress, 仅右Alt)")
        except ImportError:
            print("[WARN] keyboard 库未装,热键不可用。pip install keyboard", file=sys.stderr)

        # 系统托盘
        try:
            import pystray

            menu = pystray.Menu(
                pystray.MenuItem("状态: " + self.status_msg, lambda: None, enabled=False),
                pystray.MenuItem("退出", self._quit),
            )
            self._tray = pystray.Icon("dictation", self._make_icon(), "右 Alt 听写", menu)
            self._tray.run()
        except ImportError:
            print("[INFO] pystray 未装,无托盘图标。Ctrl+C 退出。")
            try:
                while True:
                    time.sleep(1)
            except KeyboardInterrupt:
                self._quit()

    def _quit(self, *_):
        self.recorder.stop()
        if self._tray:
            try:
                self._tray.stop()
            except Exception:
                pass
        # os._exit 而非 sys.exit:pystray 回调里 sys.exit 会抛 SystemExit 异常到消息循环
        import os
        os._exit(0)


if __name__ == "__main__":
    DictationApp().run()
