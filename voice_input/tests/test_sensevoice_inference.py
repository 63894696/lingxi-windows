"""SenseVoice Small 本地推理独立测试(2026-07-02 v0.2 实装前探活)

3 步:
  1. 加载 onnx 模型 + tokens.json
  2. WavFrontend 特征提取(mel + lfr 拼接)
  3. onnx 推理 + CTC greedy decoder

跑法:python tests/test_sensevoice_inference.py ../path/to/wav.wav
"""
import sys
import json
import time
import wave
from pathlib import Path

import numpy as np
import onnxruntime as ort


# ============================================================
# 配置(从闪电说 config.yaml 抄)
# ============================================================
MODEL_DIR = Path(r"C:/Users/Administrator/AppData/Roaming/Shandianshuo/models/sensevoice-small")
SAMPLE_RATE = 16000
N_MELS = 80
FRAME_LENGTH = 25  # ms
FRAME_SHIFT = 10   # ms
LFR_M = 7
LFR_N = 6
# WavFrontend 默认参数
PREEMPH_COEF = 0.97
CMVN_NORM = True  # SenseVoice 用 CMVN


def load_audio(wav_path: Path) -> np.ndarray:
    """读 16k mono wav,返回 float32 [-1, 1]"""
    with wave.open(str(wav_path), "rb") as wf:
        assert wf.getframerate() == SAMPLE_RATE, f"sample rate {wf.getframerate()} != {SAMPLE_RATE}"
        assert wf.getnchannels() == 1, "只支持 mono"
        assert wf.getsampwidth() == 2, "只支持 16-bit"
        n = wf.getnframes()
        audio_int16 = np.frombuffer(wf.readframes(n), dtype=np.int16)
    return audio_int16.astype(np.float32) / 32768.0


def hamming_window(L: int) -> np.ndarray:
    """hamming 窗(对标 ESPnet)"""
    return 0.54 - 0.46 * np.cos(2 * np.pi * np.arange(L) / (L - 1))


def preemphasis(x: np.ndarray, coef: float = PREEMPH_COEF) -> np.ndarray:
    """预加重"""
    if coef <= 0:
        return x
    return np.append(x[0], x[1:] - coef * x[:-1])


def mel_filterbank(n_mels: int, n_fft: int, sr: int, fmin: float = 0.0, fmax: float = None) -> np.ndarray:
    """HTK Mel 滤波器组(SenseVoice 用)"""
    if fmax is None:
        fmax = sr / 2.0
    # HTK mel scale: mel = 2595 * log10(1 + f/700)
    def hz_to_mel(f): return 2595.0 * np.log10(1.0 + f / 700.0)
    def mel_to_hz(m): return 700.0 * (10.0 ** (m / 2595.0) - 1.0)

    mel_min = hz_to_mel(fmin)
    mel_max = hz_to_mel(fmax)
    mel_points = np.linspace(mel_min, mel_max, n_mels + 2)
    hz_points = mel_to_hz(mel_points)

    bin_freq = np.linspace(0, sr / 2, n_fft // 2 + 1)
    filterbank = np.zeros((n_mels, n_fft // 2 + 1), dtype=np.float32)
    for i in range(n_mels):
        left, center, right = hz_points[i], hz_points[i + 1], hz_points[i + 2]
        for j in range(len(bin_freq)):
            f = bin_freq[j]
            if f < left or f > right:
                continue
            if f <= center:
                filterbank[i, j] = (f - left) / (center - left)
            else:
                filterbank[i, j] = (right - f) / (right - center)
    return filterbank


def extract_features(audio: np.ndarray, sr: int = SAMPLE_RATE) -> np.ndarray:
    """WavFrontend 特征提取:FFT → mel → log → CMVN → LFR 拼接

    Returns: (T, 80 * LFR_M) float32,带 LFR 拼接
    """
    # 1. 预加重
    audio = preemphasis(audio, PREEMPH_COEF)

    # 2. STFT 配置
    frame_length_samples = int(FRAME_LENGTH * sr / 1000)  # 25ms = 400 samples
    frame_shift_samples = int(FRAME_SHIFT * sr / 1000)    # 10ms = 160 samples
    n_fft = 512  # SenseVoice 默认

    # 3. Hann 窗
    window = hamming_window(frame_length_samples)

    # 4. Mel 滤波器
    fb = mel_filterbank(N_MELS, n_fft, sr)

    # 5. 分帧 + FFT
    n_frames = (len(audio) - frame_length_samples) // frame_shift_samples + 1
    if n_frames <= 0:
        return np.zeros((1, N_MELS * LFR_M), dtype=np.float32)

    frames = np.lib.stride_tricks.as_strided(
        audio,
        shape=(n_frames, frame_length_samples),
        strides=(audio.strides[0] * frame_shift_samples, audio.strides[0]),
    ).copy()
    frames = frames * window

    # FFT
    spectrum = np.fft.rfft(frames, n=n_fft, axis=1)
    power = np.abs(spectrum) ** 2  # 功率谱
    mel_power = power @ fb.T  # (n_frames, 80)
    mel_log = np.log10(np.maximum(mel_power, 1e-10))  # log mel

    # 6. CMVN(SenseVoice 训练时用,但 inference 时不做 mean norm,只做 var norm)
    if CMVN_NORM:
        mean = mel_log.mean(axis=0, keepdims=True)
        std = mel_log.std(axis=0, keepdims=True) + 1e-5
        mel_log = (mel_log - mean) / std

    # 7. LFR(Low Frame Rate)拼接:把相邻 LFR_M 帧拼成一帧,步长 LFR_N
    # 对每帧 i: output[i] = concat(features[i*lfr_n : i*lfr_n + lfr_m])
    if n_frames < LFR_M:
        # 帧太少,zero pad
        pad = np.zeros((LFR_M - n_frames, N_MELS), dtype=np.float32)
        mel_log = np.concatenate([mel_log, pad], axis=0)
        n_frames = LFR_M

    n_lfr = (n_frames - LFR_M) // LFR_N + 1
    out = np.zeros((n_lfr, N_MELS * LFR_M), dtype=np.float32)
    for i in range(n_lfr):
        start = i * LFR_N
        out[i] = mel_log[start:start + LFR_M].flatten()

    return out


def load_tokens(tokens_path: Path) -> dict:
    """加载 tokenizer(SenseVoice 用 JSON array)"""
    with open(tokens_path, "r", encoding="utf-8") as f:
        tokens = json.load(f)
    return {i: t for i, t in enumerate(tokens)}


def greedy_decode(logits: np.ndarray, tokens: dict, blank_id: int = 0) -> str:
    """CTC greedy decoder

    logits: (T, V)  log probabilities
    Returns: 解码字符串(SenseVoice 的特殊 token 标记过滤掉)
    """
    # argmax
    preds = logits.argmax(axis=-1)  # (T,)
    # 去重 + 去 blank
    result = []
    prev = -1
    for p in preds:
        if p != prev and p != blank_id:
            result.append(p)
        prev = p
    # 映射回字符串(SenseVoice 用 <|...|> 标记语言/情感/itn,这些不是文本)
    text = ""
    for idx in result:
        if idx >= len(tokens):
            continue
        tok = tokens[idx]
        if tok.startswith("<|") and tok.endswith("|>"):
            continue  # 跳过特殊 token
        if tok == "▁":
            text += " "
        else:
            text += tok.replace("▁", " ")
    return text.strip()


def main():
    if len(sys.argv) < 2:
        print("Usage: python test_sensevoice_inference.py <wav_path>")
        sys.exit(1)

    wav_path = Path(sys.argv[1])
    print(f"加载音频: {wav_path}")
    audio = load_audio(wav_path)
    print(f"  shape: {audio.shape}, duration: {len(audio)/SAMPLE_RATE:.2f}s")

    print(f"\n加载 onnx 模型: {MODEL_DIR / 'model.onnx'}")
    sess = ort.InferenceSession(
        str(MODEL_DIR / "model.onnx"),
        providers=["CPUExecutionProvider"],
    )
    print(f"  inputs: {[i.name + ': ' + str(i.shape) for i in sess.get_inputs()]}")
    print(f"  outputs: {[o.name + ': ' + str(o.shape) for o in sess.get_outputs()]}")

    tokens = load_tokens(MODEL_DIR / "tokens.json")
    print(f"  vocab size: {len(tokens)}")

    print("\n提取特征(WavFrontend)...")
    t0 = time.time()
    feats = extract_features(audio)
    feats = feats[np.newaxis, :, :]  # (1, T, 80*7)
    feats_len = np.array([feats.shape[1]], dtype=np.int32)
    print(f"  features: {feats.shape} (用了 {time.time()-t0:.2f}s)")

    print("\nonnx 推理...")
    t0 = time.time()
    # SenseVoice onnx 输入(实测 model.onnx):
    #   speech:         (B, T, 560) float32
    #   speech_lengths: (B,) int32
    #   language:       (B,) int32 — 语言 ID (0=auto, 1=zh, 2=en, 3=ja, 4=ko)
    #   textnorm:       (B,) int32 — 文本规范化 (0=off, 2=with-itn)
    feed = {
        "speech": feats.astype(np.float32),
        "speech_lengths": feats_len,
        "language": np.array([1], dtype=np.int32),     # 1=zh
        "textnorm": np.array([2], dtype=np.int32),     # 2=with-itn
    }
    outs = sess.run(None, feed)
    print(f"  inference: {time.time()-t0:.2f}s")
    for o, arr in zip(sess.get_outputs(), outs):
        print(f"  output '{o.name}': {arr.shape}")

    # CTC logits 通常在第一个 output
    logits = outs[0][0]  # (T, V)
    print(f"\nCTC logits: {logits.shape}")
    text = greedy_decode(logits, tokens)
    print(f"\n识别结果: {text!r}")


if __name__ == "__main__":
    main()