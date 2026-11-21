# -*- coding: utf-8 -*-
"""Prisir 灵犀 — 全局语音听写(2026-08-17 产品化)

右 Alt toggle：按一下开始录音（浮窗流式预览）→ 再按停止定稿上屏。
复用闪电说 SenseVoice Small 模型(本地推理,零外发)。
流式预览为伪流式（周期性对已录片段重识别），与 Linux 端对齐。

产品化 GUI:
  - 主窗口:状态灯、历史记录、设置
  - 系统托盘(叶子麦克风图标)
  - 录音悬浮窗对齐 Linux:状态灯 + 文案 + 停止/热键 + 流式预览(可拖动)

依赖: onnxruntime, sounddevice, numpy, pystray, Pillow, pyperclip, pyautogui
"""
from __future__ import annotations
import json
import os
import sys
import threading
import time
import tkinter as tk
from pathlib import Path

# 将项目 src 目录加入路径,以便导入专业名词纠错模块
_SRC_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "src")
if _SRC_DIR not in sys.path:
    sys.path.insert(0, _SRC_DIR)
from voice_input.asr.professional_corrector import get_corrector  # 专业名词纠错层

# 全局纠错器单例(首次 transcribe 时按实际环境初始化 db_path)
_corrector_instance = None

def _resolve_ciku_db() -> str:
    """定位 ciku.db: 开发态用 lingxi_ime/backend/ciku.db; 打包后用 exe 同级 models/ciku.db。"""
    # 打包/安装后: exe 同级的上级目录 models/ciku.db (NSIS 安装器放 models\ciku.db)
    cand_install = _BASE_DIR.parent / "models" / "ciku.db"
    if cand_install.exists():
        return str(cand_install)
    # 开发态: 项目根 lingxi_ime/backend/ciku.db
    cand_dev = _BASE_DIR / "lingxi_ime" / "backend" / "ciku.db"
    if cand_dev.exists():
        return str(cand_dev)
    # 回退: 默认计算路径
    return str(_BASE_DIR / "lingxi_ime" / "backend" / "ciku.db")

# pythonw 无控制台时 sys.stdout/stderr 为 None,任何 print 都会 TypeError 闪退
# → 重定向到 devnull,保证 print 安全(这就是桌面快捷方式闪退的根因)
# 关键:必须持有 devnull 文件对象引用,否则被 GC 关闭后 print 报
# "I/O operation on closed file"(运行一段时间后闪退的真凶)。
_DEVNULL_HOLDS = []
if sys.stdout is None:
    _dn = open(os.devnull, "w", encoding="utf-8", errors="replace")
    _DEVNULL_HOLDS.append(_dn)
    sys.stdout = _dn
if sys.stderr is None:
    _dn2 = open(os.devnull, "w", encoding="utf-8", errors="replace")
    _DEVNULL_HOLDS.append(_dn2)
    sys.stderr = _dn2

# 强制 UTF-8 输出,避免 Windows 控制台中文乱码
# 注意:仅当 stdout 有 .buffer(真实控制台/管道)时才包装;
# pythonw 下重定向到 devnull 的已是文本流,无 .buffer 或无需再包,跳过避免二次包装出错。
import io
if sys.stdout is not None and hasattr(sys.stdout, "buffer") and not sys.stdout.closed:
    try:
        sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
    except Exception:
        pass
if sys.stderr is not None and hasattr(sys.stderr, "buffer") and not sys.stderr.closed:
    try:
        sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding="utf-8", errors="replace")
    except Exception:
        pass

# 应用根目录: PyInstaller 冻结时用 exe 所在目录(资源随 exe 释放到 sys._MEIPASS, 见 _res),
# 否则用脚本所在目录。日志/崩溃记录写到「可写」目录(冻结时 exe 旁, Program Files 下需管理员,
# 故冻结时改写到 LOCALAPPDATA)。
_FROZEN = getattr(sys, "frozen", False)
if _FROZEN:
    _BASE_DIR = Path(sys.executable).parent
    _WRITE_DIR = Path(os.environ.get("LOCALAPPDATA", str(_BASE_DIR))) / "PrisirLingXiVoice"
    try:
        _WRITE_DIR.mkdir(parents=True, exist_ok=True)
    except Exception:
        _WRITE_DIR = _BASE_DIR
    # PyInstaller 解压资源目录(--add-data 落这)
    _RES_DIR = Path(getattr(sys, "_MEIPASS", str(_BASE_DIR)))
else:
    _BASE_DIR = Path(__file__).parent
    _WRITE_DIR = _BASE_DIR
    _RES_DIR = _BASE_DIR

def _res(name):
    """打包资源(随 --add-data 进 _MEIPASS)的绝对路径; 开发态回落脚本目录。"""
    p = _RES_DIR / name
    return p if p.exists() else (_BASE_DIR / name)

# 日志写文件,避免 print 在 GUI 线程丢失
_LOG_FILE = _WRITE_DIR / "lingxi_debug.log"
def _log(msg):
    try:
        with open(_LOG_FILE, "a", encoding="utf-8") as f:
            f.write(f"{time.strftime('%H:%M:%S')} {msg}\n")
    except Exception:
        pass

# pythonw 下未捕获异常默认静默丢失 → 安装全局钩子落盘,定位运行期崩溃
_CRASH_FILE = _WRITE_DIR / "lingxi_crash.log"
def _excepthook(exc_type, exc_value, exc_tb):
    import traceback
    try:
        with open(_CRASH_FILE, "a", encoding="utf-8") as f:
            f.write(f"\n=== {time.strftime('%Y-%m-%d %H:%M:%S')} CRASH ===\n")
            traceback.print_exception(exc_type, exc_value, exc_tb, file=f)
    except Exception:
        pass
    sys.__excepthook__(exc_type, exc_value, exc_tb)
sys.excepthook = _excepthook

def _thread_excepthook(args):
    _excepthook(args.exc_type, args.exc_value, args.exc_traceback)
if hasattr(threading, "excepthook"):
    threading.excepthook = _thread_excepthook

import numpy as np
import onnxruntime as ort
import sounddevice as sd
from PIL import Image, ImageDraw, ImageTk

# ============================================================
# 统一视觉（2026-11-20 改版：Flat / 离线工具风，对齐 ui-ux-pro-max）
# ============================================================
# 状态灯:绿=就绪 · 亮绿=录音中 · 琥珀=识别中 · 红=故障(录音绝不用红)
C = {
    "bg": "#F8FAFC",        # 页面底
    "bg2": "#FFFFFF",       # 卡片底
    "muted": "#F1F5FD",     # 弱底 / 列表行
    "primary": "#2563EB",   # 主色
    "primary_dim": "#1D4ED8",
    "secondary": "#0891B2", # 辅色
    "gold": "#2563EB",      # 兼容旧引用（原暖金改为主色蓝）
    "gold_dim": "#1D4ED8",
    "text": "#0F172A",
    "text_dim": "#475569",
    "ready": "#059669",
    "recording": "#047857",
    "busy": "#D97706",
    "error": "#DC2626",
    "border": "#E4ECFC",
    "on_primary": "#FFFFFF",
}
APP_NAME = "灵犀语音"
BALL_PX = 128  # 悬浮球逻辑像素;由 1024 原图 LANCZOS 一步缩小

FONT_UI = ("Microsoft YaHei UI", 10)
FONT_TITLE = ("Microsoft YaHei UI", 18, "bold")
FONT_SECTION = ("Microsoft YaHei UI", 11, "bold")
FONT_HINT = ("Microsoft YaHei UI", 9)
FONT_BTN = ("Microsoft YaHei UI", 10)

# ============================================================
# SenseVoice 推理(自包含,复用已验证逻辑)
# ============================================================
SAMPLE_RATE = 16000
N_MELS, FRAME_LENGTH_MS, FRAME_SHIFT_MS = 80, 25, 10
LFR_M, LFR_N, N_FFT, PREEMPH_COEF = 7, 6, 512, 0.97
# 模型目录: 优先安装目录(随安装包, Program Files\PrisirIMEoice\sensevoice-small),
# 回退开发机 Roaming 路径; PyInstaller 冻结时以 exe 所在目录为基准。
def _find_model_dir():
    import sys, os
    exe_dir = _BASE_DIR
    cands = [
        exe_dir / "sensevoice-small",                 # exe 同目录(安装器放这)
        exe_dir / "models" / "sensevoice-small",      # exe\models 子目录
        Path(os.environ.get("PRISIR_VOICE_MODEL", "")) if os.environ.get("PRISIR_VOICE_MODEL") else None,
        Path.home() / "AppData" / "Roaming" / "Shandianshuo" / "models" / "sensevoice-small",  # 开发机回退
    ]
    for c in cands:
        if c and (c / "model.onnx").exists():
            return c
    return cands[0]  # 都不在则返安装位(让上层报"模型缺失")
MODEL_DIR = _find_model_dir()


def _hamming(L):
    return 0.54 - 0.46 * np.cos(2 * np.pi * np.arange(L) / (L - 1))


def _preemphasis(x, coef=PREEMPH_COEF):
    return np.append(x[0], x[1:] - coef * x[:-1]) if coef > 0 else x


def _mel_fb(n_mels, n_fft, sr):
    def h2m(f):
        return 2595.0 * np.log10(1.0 + f / 700.0)

    def m2h(m):
        return 700.0 * (10.0 ** (m / 2595.0) - 1.0)

    hz = m2h(np.linspace(h2m(0.0), h2m(sr / 2.0), n_mels + 2))
    bin_f = np.linspace(0, sr / 2, n_fft // 2 + 1)
    fb = np.zeros((n_mels, n_fft // 2 + 1), dtype=np.float32)
    for i in range(n_mels):
        l, c, r = hz[i], hz[i + 1], hz[i + 2]
        for j in range(len(bin_f)):
            f = bin_f[j]
            if l <= f <= c:
                fb[i, j] = (f - l) / (c - l)
            elif c < f <= r:
                fb[i, j] = (r - f) / (r - c)
    return fb


def extract_features(audio, sr=SAMPLE_RATE):
    audio = _preemphasis(audio, PREEMPH_COEF)
    flen, fshift = int(FRAME_LENGTH_MS * sr / 1000), int(FRAME_SHIFT_MS * sr / 1000)
    win, fb = _hamming(flen), _mel_fb(N_MELS, N_FFT, sr)
    n_frames = max(1, (len(audio) - flen) // fshift + 1)
    if len(audio) < flen:
        audio = np.pad(audio, (0, flen - len(audio)))
    frames = np.lib.stride_tricks.as_strided(
        audio, shape=(n_frames, flen),
        strides=(audio.strides[0] * fshift, audio.strides[0])).copy() * win
    power = np.abs(np.fft.rfft(frames, n=N_FFT, axis=1)) ** 2
    mel_log = np.log10(np.maximum(power @ fb.T, 1e-10))
    mel_log = (mel_log - mel_log.mean(axis=0, keepdims=True)) / (mel_log.std(axis=0, keepdims=True) + 1e-5)
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
    result, prev = [], -1
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
        text += " " if tok == "▁" else tok.replace("▁", " ")
    return text.strip()


class SenseVoice:
    def __init__(self):
        self.session = None
        self.id2tok = None
        self._infer_lock = threading.Lock()

    def load(self):
        if not (MODEL_DIR / "model.onnx").exists() or not (MODEL_DIR / "tokens.json").exists():
            return False, f"模型未找到: {MODEL_DIR}"
        try:
            self.session = ort.InferenceSession(str(MODEL_DIR / "model.onnx"), providers=["CPUExecutionProvider"])
            toks = json.loads((MODEL_DIR / "tokens.json").read_text(encoding="utf-8"))
            self.id2tok = {i: t for i, t in enumerate(toks)}
            return True, "模型已加载"
        except Exception as e:
            return False, f"模型加载失败: {e}"

    def transcribe(self, pcm, sr=SAMPLE_RATE, language=0, textnorm=14):
        if self.session is None:
            return ""
        feats = extract_features(pcm, sr)
        feed = {
            "speech": feats[np.newaxis, :, :].astype(np.float32),
            "speech_lengths": np.array([feats.shape[0]], dtype=np.int32),
            "language": np.array([language], dtype=np.int32),
            "textnorm": np.array([textnorm], dtype=np.int32),
        }
        # 流式预览与定稿可能并发，ORT session 加锁
        with self._infer_lock:
            raw_text = greedy_decode(self.session.run(None, feed)[0][0], self.id2tok)
        # 专业名词纠错层:基于 ciku.db 词库,对 ASR 输出做专业名词替换
        global _corrector_instance
        if _corrector_instance is None:
            _corrector_instance = get_corrector(db_path=_resolve_ciku_db())
        return _corrector_instance.correct(raw_text)


# ============================================================
# 录音(callback 模式,防资源冲突)
# ============================================================
class Recorder:
    def __init__(self):
        self._frames, self._stream, self.recording, self._lock = [], None, False, threading.Lock()

    def _cb(self, indata, frames, ti, st):
        try:
            if self.recording:
                self._frames.append(indata.copy())
        except Exception:
            pass

    def start(self):
        with self._lock:
            self._stop_locked()
            self._frames, self.recording = [], True
            try:
                self._stream = sd.InputStream(samplerate=SAMPLE_RATE, channels=1, dtype="int16",
                                              blocksize=1024, callback=self._cb)
                self._stream.start()
            except Exception:
                self.recording, self._stream = False, None

    def _stop_locked(self):
        self.recording = False
        if self._stream:
            try:
                self._stream.stop(); self._stream.close()
            except Exception:
                pass
            self._stream = None

    def snapshot(self):
        """不停止录音，拷贝当前已录 PCM（供流式预览）。"""
        with self._lock:
            if not self._frames:
                return np.array([], dtype=np.float32)
            try:
                return np.concatenate(self._frames, axis=0).flatten().astype(np.float32) / 32768.0
            except Exception:
                return np.array([], dtype=np.float32)

    def stop(self):
        with self._lock:
            self._stop_locked()
            if not self._frames:
                return np.array([], dtype=np.float32)
            try:
                return np.concatenate(self._frames, axis=0).flatten().astype(np.float32) / 32768.0
            except Exception:
                return np.array([], dtype=np.float32)


# ============================================================
# 焦点窗口 + 光标打字
# ============================================================
def _fg_hwnd():
    try:
        import ctypes
        return ctypes.windll.user32.GetForegroundWindow()
    except Exception:
        return 0


def _set_fg(hwnd):
    if not hwnd:
        return
    try:
        import ctypes
        u32 = ctypes.windll.user32
        k32 = ctypes.windll.kernel32
        # 只在最小化时才 SW_RESTORE;无条件还原会把最大化窗口打回普通尺寸
        if u32.IsIconic(hwnd):
            u32.ShowWindow(hwnd, 9)
        # 跨进程 SetForegroundWindow 常失败,用 AttachThreadInput 强制附加
        cur_thread = k32.GetCurrentThreadId()
        target_thread = u32.GetWindowThreadProcessId(hwnd, None)
        if cur_thread != target_thread:
            u32.AttachThreadInput(cur_thread, target_thread, True)
        u32.SetForegroundWindow(hwnd)
        u32.SetFocus(hwnd)
        if cur_thread != target_thread:
            u32.AttachThreadInput(cur_thread, target_thread, False)
        print(f"[DEBUG] _set_fg: hwnd={hwnd} done")
    except Exception as e:
        print(f"[WARN] _set_fg: {e}")


def type_text(text):
    if not text:
        return
    try:
        import pyperclip
        pyperclip.copy(text)
        print(f"[DEBUG] type_text: clipboard set")
    except Exception as e:
        print(f"[WARN] type_text: clipboard failed: {e}")
        return
    try:
        import pyautogui
        print(f"[DEBUG] type_text: sending ctrl+v")
        time.sleep(0.05)
        pyautogui.hotkey("ctrl", "v")
        print(f"[DEBUG] type_text: ctrl+v sent")
    except Exception as e:
        print(f"[WARN] type_text: pyautogui failed: {e}")


# ============================================================
# Windows 低级键盘钩子结构(WH_KEYBOARD_LL,模块级)
# ============================================================
import ctypes as _ct
from ctypes import wintypes as _wt


class KBDLLHOOKSTRUCT(_ct.Structure):
    _fields_ = [
        ("vkCode", _wt.DWORD),
        ("scanCode", _wt.DWORD),
        ("flags", _wt.DWORD),
        ("time", _wt.DWORD),
        ("dwExtraInfo", _ct.POINTER(_ct.c_ulong)),
    ]


# 低级键盘钩子:只吞右 Alt 的松开(防止激活菜单),按下仍放行。
# 这样 GetAsyncKeyState(0xA5) 依然能看到按下,push-to-talk 沿检测不受影响,
# 而系统收不到干净的 Alt 释放,不会激活窗口菜单。
_RALT_HOOK = None
_RALT_HOOK_PROC = None
_WH_KEYBOARD_LL = 13
_WM_KEYUP = 0x0101
_WM_SYSKEYUP = 0x0105
_VK_RMENU = 0xA5


def _install_ralt_keyup_hook():
    """安装 WH_KEYBOARD_LL,在右 Alt 松开时发送无害虚拟键,防止 Alt 卡住/激活菜单。

    不吞 keyup,而是立即发送一个 VK_CONTROL 的 keydown/keyup,让系统认为
    Alt 组合了一个普通键,从而不进入"Alt 按住"状态,也不激活菜单。
    """
    global _RALT_HOOK, _RALT_HOOK_PROC
    try:
        user32 = _ct.windll.user32
        k32 = _ct.windll.kernel32

        k32.GetModuleHandleW.restype = _wt.HMODULE
        user32.SetWindowsHookExW.restype = _wt.HHOOK
        user32.CallNextHookEx.restype = _ct.c_long
        user32.CallNextHookEx.argtypes = [_wt.HHOOK, _ct.c_int, _wt.WPARAM, _wt.LPARAM]

        hmod = k32.GetModuleHandleW(None)

        HOOKPROC = _ct.WINFUNCTYPE(_ct.c_long, _ct.c_int, _wt.WPARAM, _wt.LPARAM)
        user32.SetWindowsHookExW.argtypes = [_ct.c_int, HOOKPROC, _wt.HMODULE, _wt.DWORD]

        VK_CONTROL = 0x11
        KEYEVENTF_KEYUP = 0x0002

        def _hook_proc(nCode, wParam, lParam):
            if nCode == 0:
                kb = _ct.cast(lParam, _ct.POINTER(KBDLLHOOKSTRUCT)).contents
                if kb.vkCode == _VK_RMENU and wParam in (_WM_KEYUP, _WM_SYSKEYUP):
                    # 发送无害的 Ctrl 按下/松开,中和 Alt 状态
                    # 注意:不改全局 keybd_event 的 argtypes,避免破坏 pyautogui
                    user32.keybd_event(VK_CONTROL, 0, 0, 0)
                    user32.keybd_event(VK_CONTROL, 0, KEYEVENTF_KEYUP, 0)
                    # 不吞,让原始 keyup 继续传递
            return user32.CallNextHookEx(_RALT_HOOK, nCode, wParam, lParam)

        _RALT_HOOK_PROC = HOOKPROC(_hook_proc)
        _RALT_HOOK = user32.SetWindowsHookExW(
            _WH_KEYBOARD_LL, _RALT_HOOK_PROC, hmod, 0
        )
        if not _RALT_HOOK:
            err = k32.GetLastError()
            print(f"[WARN] 右 Alt keyup 钩子安装失败,GetLastError={err}", file=sys.stderr)
            return False
        print("[OK] 右 Alt 松开已中和(防菜单/防 Alt 卡住)")
        return True
    except Exception as e:
        import traceback
        print(f"[WARN] 右 Alt keyup 钩子异常:{e}\n{traceback.format_exc()}", file=sys.stderr)
        return False


def _uninstall_ralt_keyup_hook():
    global _RALT_HOOK
    if _RALT_HOOK:
        try:
            _ct.windll.user32.UnhookWindowsHookEx(_RALT_HOOK)
        except Exception:
            pass
        _RALT_HOOK = None


# ============================================================
# 主应用
# ============================================================
class LingXiApp:
    # 流式预览：每隔多久对「已录片段」跑一次识别（伪流式；SenseVoice 非整句流式模型）
    PARTIAL_INTERVAL = 1.1
    PARTIAL_MIN_SEC = 0.7

    def __init__(self):
        self.asr = SenseVoice()
        self.recorder = Recorder()
        self.status = "init"  # init/ready/recording/busy/error
        self.status_msg = "初始化中…"
        self.history = []  # [{time, text}]
        self._alt_pressed = False
        self._target_hwnd = 0
        self._quit_flag = False
        self._tray = None
        self._win = None
        self._lamp = None
        self._status_lbl = None
        self._hist_box = None
        self._hist_inner = None
        self._float_ball = None  # 录音悬浮球
        self._lock = threading.Lock()
        self._partial_stop = threading.Event()
        self._partial_thread = None
        self._partial_text = ""
        self._partial_busy = False

    # ---------- 录音悬浮球(2026-08-17 选定第3款:agnesai 铜光叶麦克风) ----------
    # 源图 1024×1024 → LANCZOS 一步缩到 BALL_PX,窗口几何与图像同像素,禁止拉伸。
    # 光环:录音=暖金脉动 / 识别=黄 / 故障才红。
    ICON_LEAF_MIC = _res("icon_candidates/lingxi_leaf_mic.png")
    _mark_cache = None  # 缓存已打开的 RGBA 原图,避免每次脉冲重读盘

    def _load_mark(self):
        if LingXiApp._mark_cache is None:
            LingXiApp._mark_cache = Image.open(self.ICON_LEAF_MIC).convert("RGBA")
        return LingXiApp._mark_cache

    def _hex_rgb(self, hx):
        hx = hx.lstrip("#")
        return tuple(int(hx[i:i + 2], 16) for i in (0, 2, 4))

    def _glow_layer(self, color, px, boost=1.0):
        """内圈绿色柔光晕(呼吸层):集中在图标内圈,不外溢。

        径向渐变 + 高斯模糊 → 只含辉光,不含图标。
        辉光限制在内圈(R_in 到 R_out 都在图标内部),避免外圈黑边。
        """
        from PIL import ImageFilter
        r, g, b = self._hex_rgb(color)
        r, g, b = (min(255, int(v * boost)) for v in (r, g, b))
        S = 4
        big = px * S
        layer = Image.new("RGBA", (big, big), (0, 0, 0, 0))
        d = ImageDraw.Draw(layer)
        cx = big // 2
        # 辉光在图标内圈边缘:从中心偏外开始,到图标内边缘
        R_out = int(big * 0.38)   # 光晕外半径(贴近图标内边缘)
        R_in = int(big * 0.18)    # 光晕起始(中心偏外,不聚在中心)
        steps = 48
        for i in range(steps):
            t = i / (steps - 1)
            rad = int(R_in + (R_out - R_in) * t)
            alpha = int(190 * (1 - t) ** 1.6)
            if alpha <= 0:
                continue
            d.ellipse([cx - rad, cx - rad, cx + rad, cx + rad], outline=(r, g, b, alpha), width=S * 2)
        layer = layer.filter(ImageFilter.GaussianBlur(radius=big * 0.015))
        return layer.resize((px, px), Image.Resampling.LANCZOS)

    def _compose_ball(self, ring_color, glow_alpha, px):
        """层级:图标 → 呼吸辉光(最顶层,直接可见)。

        辉光放最顶层,呼吸效果直接可见;不再被图标遮挡。
        """
        px = px or BALL_PX
        # 1) 底层:图标
        base = Image.new("RGBA", (px, px), (0, 0, 0, 0))
        d = ImageDraw.Draw(base)
        ring_w = max(3, px // 32)
        pad = ring_w + 4
        try:
            inner = px - pad * 2
            mark = self._load_mark().resize((inner, inner), Image.Resampling.LANCZOS)
            mask = Image.new("L", (inner, inner), 0)
            ImageDraw.Draw(mask).ellipse([0, 0, inner - 1, inner - 1], fill=255)
            base.paste(mark, (pad, pad), mask)
        except Exception:
            d.ellipse([px // 3, px // 3, px * 2 // 3, px * 2 // 3], fill=C["gold"])
        # 2) 顶层:呼吸辉光(内圈,直接可见)
        if glow_alpha > 0:
            glow = self._glow_layer(ring_color, px, boost=1.35)
            a = glow.getchannel("A").point(lambda v: int(v * glow_alpha))
            glow.putalpha(a)
            base = Image.alpha_composite(base, glow)
        # 3) 状态细环(恢复,现在辉光在内圈,外圈干净)
        d = ImageDraw.Draw(base)
        d.ellipse([1, 1, px - 2, px - 2], outline=ring_color, width=ring_w)
        return base

    def _make_ball_image(self, ring_color, px=None, glow_alpha=0.0):
        img = self._compose_ball(ring_color, glow_alpha, px or BALL_PX)
        return ImageTk.PhotoImage(img)

    def _ball_frames(self, ring_color, px):
        """预生成一套呼吸帧(辉光从暗到亮再到暗),缓存复用,脉动只轮播不重算。

        高斯模糊合成较耗 CPU,逐帧实时算会卡;一次性算 24 帧缓存后,
        每次脉动仅切图(纯显示),几乎零开销。
        """
        key = (ring_color, px)
        cache = getattr(self, "_frame_cache", None)
        if cache is None:
            cache = self._frame_cache = {}
        if key in cache:
            return cache[key]
        import math
        N = 24
        frames = []
        for i in range(N):
            # 加大呼吸幅度: 0.2 → 1.0 → 0.2,变化更明显
            glow = 0.2 + 0.8 * (0.5 + 0.5 * math.sin(2 * math.pi * i / N))
            frames.append(ImageTk.PhotoImage(self._compose_ball(ring_color, glow, px)))
        # 稳亮帧(识别/故障用)
        steady = ImageTk.PhotoImage(self._compose_ball(ring_color, 0.55, px))
        cache[key] = (frames, steady)
        return cache[key]

    def _show_float_ball(self):
        """录音悬浮窗（对齐 Linux FloatBall）：灯 + 状态文案 + 停止/热键 + 流式预览。"""
        if self._float_ball:
            try:
                self._float_set_status("录音中（流式预览）…", recording=True)
                self._float_ball.deiconify()
                self._float_ball.lift()
            except Exception:
                pass
            return
        try:
            ball = tk.Toplevel(self._win) if self._win else tk.Toplevel()
            ball.overrideredirect(True)
            ball.attributes("-topmost", True)
            ball.attributes("-alpha", 0.98)
            ball.configure(bg=C["bg2"])
            sw = ball.winfo_screenwidth()
            win_w, win_h = 420, 118
            pos = getattr(self, "_ball_pos", None)
            if pos:
                x, y = pos
            else:
                x, y = max(8, sw - win_w - 24), 48
            ball.geometry(f"{win_w}x{win_h}+{x}+{y}")

            root = tk.Frame(ball, bg=C["bg2"], padx=10, pady=8)
            root.pack(fill=tk.BOTH, expand=True)

            top = tk.Frame(root, bg=C["bg2"])
            top.pack(fill=tk.X)

            lamp = tk.Label(top, text="●", bg=C["bg2"], fg=C["recording"],
                            font=("Microsoft YaHei UI", 11))
            lamp.pack(side=tk.LEFT, padx=(0, 6))
            ball._lamp = lamp

            status = tk.Label(
                top, text="录音中（流式预览）…", bg=C["bg2"], fg=C["text"],
                font=("Microsoft YaHei UI", 10), anchor="w",
            )
            status.pack(side=tk.LEFT, fill=tk.X, expand=True)
            ball._status = status

            stop_btn = tk.Button(
                top, text="停止", font=("Microsoft YaHei UI", 9),
                bg=C["muted"], fg=C["text"], relief=tk.FLAT, padx=8, pady=2,
                command=lambda: self.on_alt(),
            )
            stop_btn.pack(side=tk.RIGHT, padx=(4, 0))
            ball._stop = stop_btn

            hk_btn = tk.Button(
                top, text="热键", font=("Microsoft YaHei UI", 9),
                bg=C["muted"], fg=C["text"], relief=tk.FLAT, padx=8, pady=2,
                command=lambda: self._open_hotkey_settings(),
            )
            hk_btn.pack(side=tk.RIGHT)
            ball._hotkey = hk_btn

            partial = tk.Label(
                root, text="…", bg=C["bg2"], fg=C["text"],
                font=("Microsoft YaHei UI", 12), wraplength=390,
                justify=tk.LEFT, anchor="nw",
            )
            partial.pack(fill=tk.BOTH, expand=True, pady=(8, 0))
            ball._partial = partial

            tip = tk.Label(
                root, text="再按热键或点「停止」上屏", bg=C["bg2"], fg=C["text_dim"],
                font=("Microsoft YaHei UI", 8), anchor="w",
            )
            tip.pack(fill=tk.X, pady=(4, 0))
            ball._tip = tip

            self._float_ball = ball
            self._drag_data = {"x": 0, "y": 0}
            for w in (ball, root, top, lamp, status, partial, tip):
                w.bind("<Button-1>", self._ball_drag_start)
                w.bind("<B1-Motion>", self._ball_drag_move)
                w.bind("<ButtonRelease-1>", self._ball_drag_end)
        except Exception as e:
            print(f"[WARN] _show_float_ball: {e}")
            self._float_ball = None

    def _float_set_status(self, text: str, recording: bool = True):
        if not self._float_ball:
            return
        try:
            self._float_ball._status.config(text=text)
            self._float_ball._lamp.config(fg=C["recording"] if recording else C["busy"])
            if hasattr(self._float_ball, "_tip"):
                self._float_ball._tip.config(
                    text="再按热键或点「停止」上屏" if recording else "定稿识别中…"
                )
        except Exception:
            pass

    def _set_partial_text(self, text: str):
        """主线程更新流式预览。"""
        if not self._float_ball:
            return
        try:
            t = (text or "").strip() or "…"
            self._float_ball._partial.config(text=t)
        except Exception:
            pass

    def _ball_drag_start(self, event):
        self._drag_data["x"] = event.x
        self._drag_data["y"] = event.y

    def _ball_drag_move(self, event):
        if not self._float_ball:
            return
        x = self._float_ball.winfo_x() + event.x - self._drag_data["x"]
        y = self._float_ball.winfo_y() + event.y - self._drag_data["y"]
        self._float_ball.geometry(f"+{x}+{y}")

    def _ball_drag_end(self, event):
        if not self._float_ball:
            return
        self._ball_pos = (self._float_ball.winfo_x(), self._float_ball.winfo_y())

    def _hide_float_ball(self):
        if self._float_ball:
            try:
                self._float_ball.destroy()
            except Exception:
                pass
            self._float_ball = None

    # ---------- 状态 ----------
    def set_status(self, s, msg=""):
        with self._lock:
            self.status, self.status_msg = s, msg
        self._refresh_ui()

    def _refresh_ui(self):
        # 托盘
        if self._tray:
            try:
                self._tray.icon = self._make_icon()
                self._tray.title = f"{APP_NAME}: {self.status_msg}"
            except Exception:
                pass
        # 主窗口(线程安全调度到 tk 主线程)
        if self._win:
            try:
                self._win.after(0, self._update_window)
            except Exception:
                pass

    # ---------- 统一视觉图标(叶子 + 麦克风 + 状态灯,精致版) ----------
    def _make_icon(self):
        """托盘/任务栏图标:1024 原图缩到 128（多尺寸 ICO 另嵌在 exe），右下状态灯。"""
        # 64→系统再缩易糊；用 128 给托盘，任务栏走 exe 内嵌多尺寸 lingxi.ico
        out = 128
        S = 2
        canvas = out * S
        img = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
        d = ImageDraw.Draw(img)
        lamp = {
            "ready": C["ready"],
            "recording": C["recording"],
            "busy": C["busy"],
            "error": C["error"],
        }.get(self.status, "#9aa3b2")

        def sc(box):
            return [v * S for v in box]

        try:
            mark = self._load_mark().resize((canvas, canvas), Image.Resampling.LANCZOS)
            mask = Image.new("L", (canvas, canvas), 0)
            ImageDraw.Draw(mask).ellipse([0, 0, canvas - 1, canvas - 1], fill=255)
            img.paste(mark, (0, 0), mask)
        except Exception:
            d.ellipse(sc([6, 8, 42, 44]), outline=C["gold"], width=3 * S)
            d.ellipse(sc([24, 16, 34, 32]), fill=lamp)
            d.rectangle(sc([27, 30, 31, 42]), fill=lamp)
        # 状态灯相对 128 画布
        d.ellipse(
            [int(canvas * 0.68), int(canvas * 0.68), int(canvas * 0.94), int(canvas * 0.94)],
            fill=lamp,
            outline=C["bg"],
            width=max(2, S),
        )
        return img.resize((out, out), Image.Resampling.LANCZOS)

    # ---------- 听写流程(右 Alt toggle:按一下开始,再按一下停止并识别) ----------
    def on_alt(self):
        """按下沿:根据当前状态开始或停止录音。"""
        try:
            _log(f"[DEBUG] on_alt: status={self.status}")
            if self.status == "ready":
                self._target_hwnd = _fg_hwnd()
                _log(f"[DEBUG] on_alt: start recording, target={self._target_hwnd}")
                self.recorder.start()
                self._partial_text = ""
                self.set_status("recording", "录音中（流式预览）… 再按右 Alt 上屏")
                if self._win:
                    try:
                        self._win.after(0, self._show_float_ball)
                        self._win.after(50, lambda: self._set_partial_text(""))
                    except Exception:
                        pass
                self._start_partial_loop()
            elif self.status == "recording":
                _log(f"[DEBUG] on_alt: stop recording")
                self._stop_transcribe()
        except Exception as e:
            _log(f"[WARN] on_alt failed: {e}")
            self._stop_partial_loop()
            self.set_status("ready", "就绪")

    def on_alt_release(self):
        """松开沿:toggle 模式下不处理(保留钩子吞 keyup 防菜单)。"""
        pass

    def _start_partial_loop(self):
        """伪流式：录音中周期性对已录片段识别，更新悬浮预览。"""
        self._stop_partial_loop()
        self._partial_stop.clear()

        def loop():
            while not self._partial_stop.is_set() and self.status == "recording":
                if self._partial_busy:
                    time.sleep(0.15)
                    continue
                pcm = self.recorder.snapshot()
                if len(pcm) < SAMPLE_RATE * self.PARTIAL_MIN_SEC or not self._has_speech(pcm):
                    self._partial_stop.wait(self.PARTIAL_INTERVAL)
                    continue
                self._partial_busy = True
                try:
                    text = self.asr.transcribe(pcm)
                except Exception as e:
                    _log(f"[WARN] partial: {e}")
                    text = ""
                finally:
                    self._partial_busy = False
                if self._partial_stop.is_set() or self.status != "recording":
                    break
                if text and text != self._partial_text:
                    self._partial_text = text
                    if self._win:
                        try:
                            self._win.after(0, lambda t=text: self._set_partial_text(t))
                        except Exception:
                            pass
                self._partial_stop.wait(self.PARTIAL_INTERVAL)

        self._partial_thread = threading.Thread(target=loop, daemon=True)
        self._partial_thread.start()

    def _stop_partial_loop(self):
        self._partial_stop.set()
        t = self._partial_thread
        self._partial_thread = None
        if t and t.is_alive() and t is not threading.current_thread():
            t.join(timeout=0.2)

    def _stop_transcribe(self):
        try:
            self._stop_partial_loop()
            self.set_status("busy", "定稿识别中…")
            if self._win:
                try:
                    self._win.after(0, lambda: self._float_set_status("定稿识别中…", recording=False))
                    if self._partial_text:
                        self._win.after(0, lambda: self._set_partial_text(self._partial_text))
                except Exception:
                    pass
            pcm = self.recorder.stop()
            if len(pcm) < SAMPLE_RATE * 0.5:
                if self._win:
                    try:
                        self._win.after(0, self._hide_float_ball)
                    except Exception:
                        pass
                self.set_status("ready", "就绪(录音太短)")
                return
            # VAD 前奏检测: 检查是否有有效语音(能量阈值)
            if not self._has_speech(pcm):
                if self._win:
                    try:
                        self._win.after(0, self._hide_float_ball)
                    except Exception:
                        pass
                self.set_status("ready", "就绪(未检测到语音)")
                return
            text = self.asr.transcribe(pcm)
            if not text:
                text = self._partial_text  # 定稿失败时回退流式结果
            _log(f"[DEBUG] _stop_transcribe: raw text={text!r}")
            # 1) 字典纠偏:ASR 输出后术语替换
            text = self._apply_dict(text)
            _log(f"[DEBUG] _stop_transcribe: after dict={text!r}")
            # 2) 片段替换:指令词 → 预设文本
            text = self._apply_snippets(text)
            _log(f"[DEBUG] _stop_transcribe: after snippets={text!r}")
            # 3) 风格整理:本地规则(轻度/深度)
            text = self._apply_style(text)
            _log(f"[DEBUG] _stop_transcribe: after style={text!r}")
            if text:
                if self._target_hwnd:
                    _set_fg(self._target_hwnd)
                    time.sleep(0.1)
                type_text(text)
                self._add_history(text)
                self.set_status("ready", f"✓ {text[:20]}")
            else:
                self.set_status("ready", "就绪(未识别到内容)")
            # 完成 → 隐藏悬浮球
            if self._win:
                try:
                    self._win.after(0, self._hide_float_ball)
                except Exception:
                    pass
        except Exception:
            self._stop_partial_loop()
            if self._win:
                try:
                    self._win.after(0, self._hide_float_ball)
                except Exception:
                    pass
            self.set_status("ready", "就绪")

    def _has_speech(self, pcm, threshold=0.01):
        """VAD 前奏检测: 检查音频是否有有效语音(能量阈值)"""
        import numpy as np
        if len(pcm) == 0:
            return False
        # 计算 RMS 能量
        rms = np.sqrt(np.mean(pcm ** 2))
        # 检查是否有超过阈值的片段
        return rms > threshold

    def _add_history(self, text):
        self.history.insert(0, {"time": time.strftime("%H:%M:%S"), "text": text})
        self.history = self.history[:50]

    # ---------- 字典/片段/风格(2026-08-18 新增) ----------
    def _apply_dict(self, text):
        """ASR 输出后术语替换(字典纠偏)"""
        try:
            from lingxi_config import USER_DICT
            _log(f"[DEBUG] _apply_dict: input={text!r}, dict={USER_DICT}")
            for wrong, right in USER_DICT.items():
                if wrong in text:
                    _log(f"[DEBUG] dict hit: {wrong} -> {right}")
                    text = text.replace(wrong, right)
            _log(f"[DEBUG] _apply_dict: output={text!r}")
            return text
        except Exception as e:
            _log(f"[WARN] dict failed: {e}")
            return text

    def _apply_snippets(self, text):
        """指令词 → 预设文本(片段展开)"""
        try:
            from lingxi_config import USER_SNIPPETS
            for trigger, snippet in USER_SNIPPETS.items():
                if trigger in text:
                    print(f"[DEBUG] snippet hit: {trigger} -> {snippet[:20]}...")
                    text = text.replace(trigger, snippet)
            return text
        except Exception as e:
            print(f"[WARN] snippet failed: {e}")
            return text

    def _apply_style(self, text):
        """输出风格整理(本地规则)"""
        try:
            from lingxi_config import STYLE_CONFIG
            level = STYLE_CONFIG.get("level", "light")
            tone = STYLE_CONFIG.get("tone", "neutral")
            if level == "raw":
                return text
            # 轻度整理:去口语词、补标点
            if level in ("light", "deep"):
                # 去常见口语填充词
                fillers = ["嗯", "啊", "呃", "那个", "就是", "然后"]
                for f in fillers:
                    text = text.replace(f, "")
                # 简单标点规整
                text = text.replace("  ", " ").strip()
                if text and not text.endswith(("。", "!", "?", ".", "!", "?")):
                    text += "。"
            # 深度整理:接大模型(暂未启用,预留接口)
            if level == "deep" and STYLE_CONFIG.get("llm_polish"):
                # TODO: 接入 Prisir 大模型润色
                pass
            return text
        except Exception:
            return text

    def _delete_history(self, idx):
        # 从数据源删除(2026-08-17 修复:之前只清 UI,刷新又从 history 重填 → 删了又出现)
        if 0 <= idx < len(self.history):
            self.history.pop(idx)
        self._update_window()

    def _copy_history(self, idx):
        if 0 <= idx < len(self.history):
            try:
                import pyperclip
                pyperclip.copy(self.history[idx]["text"])
            except Exception:
                pass

    def _clear_history(self):
        self.history = []
        self._update_window()

    def _open_hotkey_settings(self):
        """打开热键设置窗口（点击按钮捕获按键）"""
        if hasattr(self, "_hotkey_win") and self._hotkey_win:
            try:
                self._hotkey_win.deiconify()
                self._hotkey_win.lift()
                return
            except Exception:
                pass

        win = tk.Toplevel(self._win)
        self._hotkey_win = win
        win.title("热键设置")
        win.geometry("400x280")
        win.configure(bg=C["bg"])
        win.resizable(False, False)
        win.protocol("WM_DELETE_WINDOW", win.withdraw)

        current_vk = self._get_current_trigger_key()
        current_name = self._vk_to_name(current_vk)

        card = tk.Frame(win, bg=C["bg2"], highlightbackground=C["border"], highlightthickness=1)
        card.pack(fill=tk.BOTH, expand=True, padx=16, pady=16)

        tk.Label(card, text="听写热键", bg=C["bg2"], fg=C["text"],
                 font=FONT_SECTION).pack(anchor=tk.W, padx=18, pady=(16, 4))
        tk.Label(
            card,
            text="按一下开始录音，再按一下结束并识别。\n可与输入法拼音/五笔激活键错开。",
            bg=C["bg2"], fg=C["text_dim"], font=FONT_HINT, justify=tk.LEFT,
        ).pack(anchor=tk.W, padx=18, pady=(0, 12))

        self._hotkey_btn = tk.Button(
            card,
            text=current_name,
            command=self._start_key_capture,
            bg=C["primary"], fg=C["on_primary"], relief=tk.FLAT,
            font=("Microsoft YaHei UI", 15, "bold"),
            cursor="hand2", activebackground=C["primary_dim"],
            activeforeground=C["on_primary"], padx=20, pady=14,
        )
        self._hotkey_btn.pack(fill=tk.X, padx=18, pady=(0, 8))

        tk.Label(card, text="点击上方按钮，然后按下新热键", bg=C["bg2"], fg=C["text_dim"],
                 font=FONT_HINT).pack(anchor=tk.W, padx=18)

        self._hotkey_status = tk.Label(card, text="", bg=C["bg2"], fg=C["ready"],
                                        font=FONT_HINT)
        self._hotkey_status.pack(anchor=tk.W, padx=18, pady=(10, 16))

    def _get_current_trigger_key(self):
        """从配置文件读取当前触发键"""
        try:
            from lingxi_config import TRIGGER_KEY
            return TRIGGER_KEY
        except Exception:
            return 0xA5  # 默认右 Alt

    def _vk_to_name(self, vk):
        """虚拟键码转名称"""
        names = {
            0xA5: "右 Alt",
            0x12: "右 Alt",  # Tkinter keycode 把 Alt 映射为 0x12
            0x70: "F1", 0x71: "F2", 0x72: "F3", 0x73: "F4",
            0x74: "F5", 0x75: "F6", 0x76: "F7", 0x77: "F8",
            0x78: "F9", 0x79: "F10", 0x7A: "F11", 0x7B: "F12",
        }
        return names.get(vk, f"VK_{vk:#x}")

    def _start_key_capture(self):
        """开始捕获按键"""
        self._hotkey_btn.config(text="请按新热键…", bg=C["busy"], fg=C["on_primary"])
        self._hotkey_status.config(text="等待按键…", fg=C["busy"])
        self._hotkey_win.bind("<Key>", self._on_key_capture)
        self._hotkey_win.focus_set()

    def _on_key_capture(self, event):
        """捕获按键"""
        vk = event.keycode
        # Tkinter 把 Alt 映射为 0x12，需要转成 Windows 的 VK_RMENU (0xA5)
        if vk == 0x12:
            vk = 0xA5
        name = self._vk_to_name(vk)
        self._save_trigger_key(vk)
        self._reload_trigger_key(vk)
        self._hotkey_btn.config(text=name, bg=C["primary"], fg=C["on_primary"])
        self._hotkey_status.config(text=f"已设置为 {name}，立即生效", fg=C["ready"])
        self._hotkey_win.unbind("<Key>")
        self._update_trigger_hint(name)

    def _reload_trigger_key(self, vk):
        """重新加载触发键，立即生效"""
        # 更新实例变量，让 poll 线程读到新值
        self._trigger_key_override = vk
        # 同时更新配置文件
        self._save_trigger_key(vk)

    def _save_trigger_key(self, vk):
        """保存触发键到配置文件"""
        try:
            config_path = _res("lingxi_config.py")
            content = config_path.read_text(encoding="utf-8")
            import re
            content = re.sub(r"TRIGGER_KEY = 0x[0-9A-Fa-f]+", f"TRIGGER_KEY = 0x{vk:02X}", content)
            config_path.write_text(content, encoding="utf-8")
        except Exception as e:
            self._hotkey_status.config(text=f"保存失败: {e}")

    def _update_trigger_hint(self, name):
        """更新主窗口提示文字"""
        hint = getattr(self, "_trigger_hint_lbl", None)
        if hint is not None:
            try:
                hint.config(text=f"按一下 {name} 开始听写，再按一下结束并识别 · 本地 SenseVoice")
                return
            except Exception:
                pass
        for widget in self._win.winfo_children():
            if isinstance(widget, tk.Label):
                t = widget.cget("text")
                if "听写" in t or "说话" in t:
                    widget.config(text=f"按一下 {name} 开始听写，再按一下结束并识别 · 本地 SenseVoice")
                    break

    def _apply_trigger_key(self):
        """应用触发按键设置"""
        key_map = {
            "右 Alt": 0xA5,
            "F1": 0x70, "F2": 0x71, "F3": 0x72, "F4": 0x73,
            "F5": 0x74, "F6": 0x75, "F7": 0x76, "F8": 0x77,
            "F9": 0x78, "F10": 0x79, "F11": 0x7A, "F12": 0x7B,
        }
        key_name = self._trigger_key_var.get()
        vk = key_map.get(key_name, 0xA5)
        # 更新配置文件
        try:
            config_path = _res("lingxi_config.py")
            content = config_path.read_text(encoding="utf-8")
            import re
            content = re.sub(r"TRIGGER_KEY = 0x[0-9A-Fa-f]+", f"TRIGGER_KEY = 0x{vk:02X}", content)
            config_path.write_text(content, encoding="utf-8")
            self.set_status("ready", f"触发按键已改为 {key_name}，重启生效")
        except Exception as e:
            self.set_status("error", f"设置失败: {e}")

    # ---------- 主窗口 ----------
    def _update_window(self):
        if not self._win:
            return
        lamp_color = {
            "ready": C["ready"],
            "recording": C["recording"],
            "busy": C["busy"],
            "error": C["error"],
        }.get(self.status, "#9aa3b2")
        self._lamp.itemconfig(self._lamp_dot, fill=lamp_color)
        self._status_lbl.config(text=self.status_msg)
        # 重建历史列表(每行:时间 + 文本 + 复制/删除按钮)
        for w in self._hist_inner.winfo_children():
            w.destroy()
        for i, h in enumerate(self.history):
            row_bg = C["muted"] if i % 2 == 0 else C["bg2"]
            row = tk.Frame(self._hist_inner, bg=row_bg)
            row.pack(fill=tk.X, pady=2, padx=6)
            tk.Label(row, text=h["time"], bg=row_bg, fg=C["text_dim"],
                     font=FONT_HINT).pack(side=tk.LEFT, padx=(10, 8), pady=8)
            tk.Label(row, text=h["text"], bg=row_bg, fg=C["text"],
                     font=("Microsoft YaHei UI", 11), wraplength=250, justify=tk.LEFT,
                     anchor="w").pack(side=tk.LEFT, fill=tk.X, expand=True, pady=8)
            tk.Button(row, text="删除", command=lambda i=i: self._delete_history(i),
                      bg=row_bg, fg=C["error"], relief=tk.FLAT, font=FONT_HINT,
                      cursor="hand2", padx=8, activebackground=row_bg).pack(side=tk.RIGHT, padx=2)
            tk.Button(row, text="复制", command=lambda i=i: self._copy_history(i),
                      bg=row_bg, fg=C["primary"], relief=tk.FLAT, font=FONT_HINT,
                      cursor="hand2", padx=8, activebackground=row_bg).pack(side=tk.RIGHT, padx=2)

    def open_window(self, *_):
        if self._win:
            try:
                self._win.deiconify()
                self._win.lift()
            except Exception:
                pass
            return
        win = tk.Tk()
        self._win = win
        win.title(APP_NAME)
        win.geometry("520x680")
        win.configure(bg=C["bg"])
        win.protocol("WM_DELETE_WINDOW", win.withdraw)  # 关闭=最小化到托盘
        try:
            ico = _res("lingxi.ico")
            if ico.exists():
                win.iconbitmap(default=str(ico))
        except Exception:
            pass

        # ── 品牌头 ─────────────────────────────────────────────
        brand = tk.Frame(win, bg=C["bg"])
        brand.pack(fill=tk.X, padx=20, pady=(20, 4))
        tk.Label(brand, text=APP_NAME, bg=C["bg"], fg=C["text"],
                 font=FONT_TITLE).pack(anchor=tk.W)
        tk.Label(brand, text="BabelSpan · 本地听写", bg=C["bg"], fg=C["text_dim"],
                 font=FONT_HINT).pack(anchor=tk.W, pady=(2, 0))

        # ── 状态卡 ─────────────────────────────────────────────
        status_card = tk.Frame(win, bg=C["bg2"], highlightbackground=C["border"], highlightthickness=1)
        status_card.pack(fill=tk.X, padx=20, pady=(12, 8))
        head = tk.Frame(status_card, bg=C["bg2"])
        head.pack(fill=tk.X, padx=16, pady=(14, 6))
        self._lamp = tk.Canvas(head, width=28, height=28, bg=C["bg2"], highlightthickness=0)
        self._lamp.pack(side=tk.LEFT)
        self._lamp_dot = self._lamp.create_oval(4, 4, 24, 24, fill=C["ready"], outline="")
        self._status_lbl = tk.Label(head, text=self.status_msg, bg=C["bg2"], fg=C["text"],
                                    font=FONT_SECTION)
        self._status_lbl.pack(side=tk.LEFT, padx=10)

        trigger_name = self._vk_to_name(self._get_current_trigger_key())
        self._trigger_hint_lbl = tk.Label(
            status_card,
            text=f"按一下 {trigger_name} 开始听写，再按一下结束并识别 · 本地 SenseVoice",
            bg=C["bg2"], fg=C["text_dim"], font=FONT_HINT, justify=tk.LEFT,
        )
        self._trigger_hint_lbl.pack(anchor=tk.W, padx=16, pady=(0, 14))

        # ── 设置卡 ─────────────────────────────────────────────
        settings_card = tk.Frame(win, bg=C["bg2"], highlightbackground=C["border"], highlightthickness=1)
        settings_card.pack(fill=tk.X, padx=20, pady=(0, 8))
        tk.Label(settings_card, text="设置", bg=C["bg2"], fg=C["text"],
                 font=FONT_SECTION).pack(anchor=tk.W, padx=16, pady=(14, 6))
        tk.Button(
            settings_card, text="热键设置", command=self._open_hotkey_settings,
            bg=C["primary"], fg=C["on_primary"], relief=tk.FLAT, font=FONT_BTN,
            cursor="hand2", padx=16, pady=8,
            activebackground=C["primary_dim"], activeforeground=C["on_primary"],
        ).pack(anchor=tk.W, padx=16, pady=(0, 14))

        # ── 历史卡 ─────────────────────────────────────────────
        hist_card = tk.Frame(win, bg=C["bg2"], highlightbackground=C["border"], highlightthickness=1)
        hist_card.pack(fill=tk.BOTH, expand=True, padx=20, pady=(0, 8))
        hist_head = tk.Frame(hist_card, bg=C["bg2"])
        hist_head.pack(fill=tk.X, padx=12, pady=(12, 4))
        tk.Label(hist_head, text="历史记录", bg=C["bg2"], fg=C["text"],
                 font=FONT_SECTION).pack(side=tk.LEFT)
        tk.Button(hist_head, text="清空", command=self._clear_history,
                  bg=C["bg2"], fg=C["text_dim"], relief=tk.FLAT, font=FONT_HINT,
                  cursor="hand2", padx=8, activebackground=C["muted"]).pack(side=tk.RIGHT)

        box_frame = tk.Frame(hist_card, bg=C["border"])
        box_frame.pack(fill=tk.BOTH, expand=True, padx=12, pady=(0, 12))
        canvas = tk.Canvas(box_frame, bg=C["bg2"], highlightthickness=0)
        scroll = tk.Scrollbar(box_frame, orient=tk.VERTICAL, command=canvas.yview)
        self._hist_inner = tk.Frame(canvas, bg=C["bg2"])
        self._hist_inner.bind("<Configure>", lambda e: canvas.configure(scrollregion=canvas.bbox("all")))
        canvas.create_window((0, 0), window=self._hist_inner, anchor="nw", width=450)
        canvas.configure(yscrollcommand=scroll.set)
        canvas.pack(side=tk.LEFT, fill=tk.BOTH, expand=True, padx=1, pady=1)
        scroll.pack(side=tk.RIGHT, fill=tk.Y)
        canvas.bind_all("<MouseWheel>", lambda e: canvas.yview_scroll(int(-e.delta / 120), "units"))

        # ── 底栏 ───────────────────────────────────────────────
        btns = tk.Frame(win, bg=C["bg"])
        btns.pack(fill=tk.X, padx=20, pady=(4, 18))
        tk.Button(btns, text="最小化到托盘", command=win.withdraw,
                  bg=C["muted"], fg=C["primary"], relief=tk.FLAT, padx=14, pady=8,
                  font=FONT_BTN, cursor="hand2",
                  activebackground=C["border"]).pack(side=tk.LEFT)
        tk.Button(btns, text="退出", command=self._quit,
                  bg=C["muted"], fg=C["error"], relief=tk.FLAT, padx=14, pady=8,
                  font=FONT_BTN, cursor="hand2",
                  activebackground=C["border"]).pack(side=tk.RIGHT)

        self._update_window()
        win.mainloop()

    # ---------- 运行 ----------
    def _start_hotkey(self):
        """自定义热键 — toggle 模式(2026-08-18 支持 F1-F12)

        按一下开始录音、出悬浮球;再按一下停止并识别。
        用 GetAsyncKeyState(TRIGGER_KEY) 轮询,支持右 Alt 或 F1-F12。
        """
        import ctypes
        from lingxi_config import TRIGGER_KEY
        user32 = ctypes.windll.user32
        VK_TRIGGER = TRIGGER_KEY

        def poll():
            was_down = False
            _log(f"[DEBUG] poll: started, trigger={VK_TRIGGER:#x}")
            while not self._quit_flag:
                try:
                    # 支持动态切换触发键
                    trigger = getattr(self, "_trigger_key_override", VK_TRIGGER)
                    down = (user32.GetAsyncKeyState(trigger) & 0x8000) != 0
                    if down and not was_down:
                        was_down = True
                        _log("[DEBUG] poll: trigger down detected")
                        threading.Thread(target=self.on_alt, daemon=True).start()
                    elif not down and was_down:
                        was_down = False
                        _log("[DEBUG] poll: trigger up detected")
                    time.sleep(0.03)
                except Exception as e:
                    _log(f"[WARN] poll error: {e}")
                    time.sleep(0.1)

        self._quit_flag = False
        threading.Thread(target=poll, daemon=True).start()
        print(f"[OK] 热键 {VK_TRIGGER:#x} toggle 模式已就绪(按一下开始/再按一下停止)")

    # ---------- 单实例(2026-08-17):防止重复启动互相干扰 ----------
    # 桌面快捷方式若重复双击,不再起第二个实例,而是把已有实例的主窗口弹到前台。
    # 机制: 命名互斥体(判断是否已运行) + 命名事件(通知已有实例弹出窗口)。
    _MUTEX_NAME = "PrisirLingXi_SingleInstance_Mutex"
    _POPUP_EVENT = "PrisirLingXi_PopupWindow_Event"
    # 拼音 IME 状态栏「语音」按钮触发(命名事件,复用 popup 同款监听模式)。
    _VOICE_EVENT = "PrisirLingXi_VoiceToggle_Event"

    def _acquire_single_instance(self):
        """获取单实例锁。若已有实例在跑 → 通知其弹窗并返回 False(本进程应退出)。
        例外:本进程是「重启接力」进程(带 LINGXI_RESTARTER=1)时,旧实例正在退出,
        互斥体短暂仍被持有 → 轮询等待其释放(最多 ~3s)再接管,而不是误判重复启动自杀。"""
        import ctypes
        k32 = ctypes.windll.kernel32
        ERROR_ALREADY_EXISTS = 183
        is_restarter = os.environ.get("LINGXI_RESTARTER") == "1"
        # 1) 创建/打开命名互斥体
        self._mutex = k32.CreateMutexW(None, False, self._MUTEX_NAME)
        already = (k32.GetLastError() == ERROR_ALREADY_EXISTS)
        # 2) 创建命名事件(用于"重复启动→弹出已有窗口"通知)
        self._popup_event = k32.CreateEventW(None, False, False, self._POPUP_EVENT)
        if already and is_restarter:
            # 等旧实例 _quit/os._exit 释放互斥体后重试
            for _ in range(30):  # 30 * 100ms = 3s
                time.sleep(0.1)
                self._mutex = k32.CreateMutexW(None, False, self._MUTEX_NAME)
                if k32.GetLastError() != ERROR_ALREADY_EXISTS:
                    already = False
                    break
        if already:
            # 已有实例:发信号让它弹窗,本进程退出
            try:
                ev = k32.OpenEventW(0x0002, False, self._POPUP_EVENT)  # EVENT_MODIFY_STATE
                if ev:
                    k32.SetEvent(ev)
            except Exception:
                pass
            return False
        # 首次实例:后台线程监听弹窗事件
        threading.Thread(target=self._popup_listener, daemon=True).start()
        # 拼音 IME 语音按钮:命名事件触发 on_alt(开始/停止听写),复用现有 toggle 逻辑。
        try:
            self._voice_event = k32.CreateEventW(None, False, False, self._VOICE_EVENT)
            threading.Thread(target=self._voice_listener, daemon=True).start()
        except Exception as e:
            _log(f"[WARN] 语音 IPC 事件创建失败:{e}")
        return True

    def _voice_listener(self):
        """监听 PrisirLingXi_VoiceToggle_Event:拼音状态栏「语音」按钮点击 → 触发听写 toggle。"""
        import ctypes
        k32 = ctypes.windll.kernel32
        while not self._quit_flag:
            try:
                r = k32.WaitForSingleObject(self._voice_event, 500)
                if r == 0:  # WAIT_OBJECT_0
                    _log("[IPC] voice toggle event -> on_alt")
                    try:
                        if self._win:
                            self._win.after(0, self.on_alt)
                    except Exception:
                        pass
            except Exception:
                break

    def _popup_listener(self):
        import ctypes
        k32 = ctypes.windll.kernel32
        while not self._quit_flag:
            try:
                r = k32.WaitForSingleObject(self._popup_event, 500)
                if r == 0:  # WAIT_OBJECT_0
                    # 有重复启动请求 → 把主窗口弹到前台
                    try:
                        if self._win:
                            self._win.after(0, self._bring_to_front)
                    except Exception:
                        pass
            except Exception:
                break

    def _bring_to_front(self):
        if not self._win:
            return
        try:
            self._win.deiconify()
            self._win.lift()
            self._win.attributes("-topmost", True)
            self._win.after(200, lambda: self._win.attributes("-topmost", False))
            self._win.focus_force()
        except Exception:
            pass

    def run(self):
        # 单实例检查:已运行则弹窗退出,不抢热键/托盘/麦克风
        try:
            if not self._acquire_single_instance():
                print("[INFO] 已有 Prisir 灵犀 在运行,已弹出其窗口,本次启动退出")
                return
        except Exception as e:
            print(f"[WARN] 单实例检查失败(继续启动):{e}")

        ok, msg = self.asr.load()
        if not ok:
            self.set_status("error", msg)
            print(f"[ERROR] {msg}", file=sys.stderr)
        else:
            self.set_status("ready", "就绪 — 按住右 Alt 说话")
            print(f"[OK] {APP_NAME} {msg}")
        _log(f"[BOOT] asr.load() ok={ok} msg={msg}")

        # 注册右 Alt 热键(已验证方案)
        self._start_hotkey()
        # 暂时禁用钩子,测试是否是钩子干扰了热键检测
        # _install_ralt_keyup_hook()

        # 托盘(2026-08-17 图标不出现修复):
        # pystray.Icon.run() 会跑自己的消息循环并阻塞所在线程 —— 放 daemon 线程时
        # 与 tkinter 主循环冲突,导致图标根本不显示。改用 run_detached():
        # pystray 自己起后台线程处理消息循环,不阻塞 tkinter,图标正常显示。
        def setup_tray():
            import pystray
            menu = pystray.Menu(
                pystray.MenuItem("打开主窗口", lambda: threading.Thread(target=self.open_window, daemon=True).start(), default=True),
                pystray.MenuItem("重启", lambda *_: threading.Thread(target=self._restart, daemon=True).start()),
                pystray.MenuItem("退出", self._quit),
            )
            self._tray = pystray.Icon("lingxi", self._make_icon(), APP_NAME, menu)
            self._tray.run_detached()  # 自带后台消息循环,不阻塞 tkinter

        try:
            setup_tray()
            print("[OK] 托盘图标已启动")
        except Exception as e:
            print(f"[WARN] 托盘启动失败:{e}", file=sys.stderr)
        # 主窗口在主线程启动
        self.open_window()

    def _quit(self, *_):
        self._quit_flag = True
        _uninstall_ralt_keyup_hook()
        self.recorder.stop()
        if self._tray:
            try:
                self._tray.stop()
            except Exception:
                pass
        os._exit(0)

    def _restart(self, *_):
        """重启输入法:意外卡死时复位工作状态(2026-08-21)。
        先 spawn 新进程,再退本进程;单实例互斥体让新进程接管热键/托盘/麦克风。"""
        try:
            import subprocess
            env = os.environ.copy()
            env["LINGXI_RESTARTER"] = "1"
            subprocess.Popen(
                [sys.executable, os.path.abspath(__file__)],
                cwd=os.path.dirname(os.path.abspath(__file__)),
                env=env, close_fds=True)
            print("[OK] 重启:已启动新进程")
        except Exception as e:
            print(f"[WARN] 重启启动新进程失败:{e}", file=sys.stderr)
        time.sleep(0.3)  # 稍候再退,让新进程先起
        self._quit()


if __name__ == "__main__":
    LingXiApp().run()
