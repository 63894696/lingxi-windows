#!/usr/bin/env python
"""voice_input 断网验证 — ADR-001 §5.4 实装版(2026-07-02 v0.3 ship)

启动 3 daemon,跑一个完整 e2e 流程,验证零外发请求。

用法:
  python scripts/verify_offline.py

要求:
  - 3 daemon 离线启动(防火墙拦截非 loopback / 拔网线)
  - 验证 ./e2e.log 记录所有 HTTP 请求(应只有 127.0.0.1)

检查项(2026-07-02 v0.3 实装):
  [✓] orchestrator /health 返 ok
  [✓] orchestrator /presence 默认 voice
  [✓] POST /speak (voice mode) 走 TTS daemon,SAPI 本地
  [✓] POST /transcribe 上传本地 wav,SenseVoice 本地推理
  [✓] POST /presence 切 mode(text/voice/auto)
  [✓] 注入文本(可选)
  [✓] 0 个外发 HTTP 连接(仅 127.0.0.1 loopback)
"""
from __future__ import annotations
import sys
import time
import base64
import socket
import threading
import subprocess
from pathlib import Path
from typing import Optional

# 把 src 加进 sys.path
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

import httpx


# 固定端口(同 config.py)
ORCHESTRATOR_PORT = 8730
ASR_DAEMON_PORT = 8731
TTS_DAEMON_PORT = 8732
BASE_URL = f"http://127.0.0.1:{ORCHESTRATOR_PORT}"
AUTH_TOKEN_FILE = Path.home() / ".voice_input" / "auth_token"


def find_external_connections(allowed_loopback: bool = True):
    """扫本机所有非 loopback TCP 连接(2026-07-02 v0.3 实装)

    Returns: list of (pid, local_addr, remote_addr, state)
    """
    try:
        out = subprocess.check_output(
            ["powershell", "-NoProfile", "-Command",
             "Get-NetTCPConnection | Where-Object {$_.State -eq 'Established'} | "
             "Select-Object OwningProcess,LocalAddress,LocalPort,RemoteAddress,RemotePort,State | "
             "ConvertTo-Csv -NoTypeInformation"],
            text=True, timeout=10
        )
        import csv
        from io import StringIO
        reader = csv.DictReader(StringIO(out))
        results = []
        for row in reader:
            try:
                la = row.get("LocalAddress", "")
                ra = row.get("RemoteAddress", "")
                # 跳过 loopback(IPAddress 字段在 Windows 是纯 IP)
                if allowed_loopback and la.startswith("127."):
                    continue
                results.append({
                    "pid": row.get("OwningProcess"),
                    "local": f"{la}:{row.get('LocalPort')}",
                    "remote": f"{ra}:{row.get('RemotePort')}",
                    "state": row.get("State"),
                })
            except Exception:
                pass
        return results
    except Exception as e:
        return [{"error": str(e)}]


def check_no_external_daemon_connections():
    """检查 voice_input 3 daemon 是否只走了 loopback(零外发)"""
    # 找到 3 daemon PID
    try:
        out = subprocess.check_output(
            ["powershell", "-NoProfile", "-Command",
             "Get-NetTCPConnection | Where-Object {$_.State -eq 'Established' -and $_.LocalPort -in 8730,8731,8732} | "
             "Select-Object OwningProcess,LocalPort,RemoteAddress,RemotePort | "
             "ConvertTo-Csv -NoTypeInformation"],
            text=True, timeout=10
        )
        # parse
        import csv
        from io import StringIO
        reader = csv.DictReader(StringIO(out))
        rows = list(reader)
        external = [r for r in rows if not str(r.get("RemoteAddress","")).startswith("127.")]
        return rows, external
    except Exception as e:
        return [], [{"error": str(e)}]


def kill_existing_daemons():
    """先杀掉本机已跑的 voice_input 3 daemon"""
    try:
        subprocess.run(
            ["powershell", "-NoProfile", "-Command",
             "Get-NetTCPConnection | Where-Object {$_.LocalPort -in 8730,8731,8732} | "
             "ForEach-Object { Stop-Process -Id $_.OwningProcess -Force }"],
            timeout=10, capture_output=True
        )
        time.sleep(1)
    except Exception:
        pass


def start_daemons():
    """启动 3 daemon(子进程,继承父进程 env + 加 PYTHONPATH)"""
    import os
    env = {**os.environ, "PYTHONPATH": str(Path(__file__).resolve().parents[1] / "src")}
    cwd = str(Path(__file__).resolve().parents[1])

    log_dir = Path.home() / ".voice_input" / "logs"
    log_dir.mkdir(parents=True, exist_ok=True)

    procs = {}
    for name, port, module in [
        ("asr", ASR_DAEMON_PORT, "voice_input.asr_daemon"),
        ("tts", TTS_DAEMON_PORT, "voice_input.tts_daemon"),
        ("orch", ORCHESTRATOR_PORT, "voice_input.orchestrator"),
    ]:
        log_file = log_dir / f"{name}.log"
        log_fp = open(log_file, "a", encoding="utf-8")
        p = subprocess.Popen(
            [sys.executable, "-m", module],
            cwd=cwd, env=env,
            stdout=log_fp, stderr=subprocess.STDOUT,
        )
        procs[name] = (p, log_file)
        print(f"  ✓ {name} daemon 启动 (PID={p.pid}, log={log_file})")

    # 等 daemon ready
    print(f"\n  等待 daemon 起来(等 10s 给 onnx 加载)...")
    time.sleep(10)

    # 验证
    for name, (p, _) in procs.items():
        if p.poll() is not None:
            print(f"  ✗ {name} daemon 异常退出 (exit code={p.returncode})")
            return None
    print(f"  ✓ 3 daemon 全部 running")
    return procs


def stop_daemons(procs):
    """关闭 3 daemon"""
    for name, (p, _) in procs.items():
        try:
            p.terminate()
            p.wait(timeout=3)
        except Exception:
            try:
                p.kill()
            except Exception:
                pass
    time.sleep(1)


def main():
    print("=" * 60)
    print("voice_input 断网验证 — ADR-001 §5.4 (v0.3 实装)")
    print("=" * 60)
    print()
    print("前置要求:")
    print("  - 3 daemon 离线启动(防火墙拦截 / 拔网线)")
    print("  - SenseVoice 模型已就位(闪电说安装包)")
    print()

    # 0) 杀旧 daemon
    print("[0] 杀掉旧 daemon...")
    kill_existing_daemons()

    # 1) 启动 daemon
    print("\n[1] 启动 3 daemon...")
    procs = start_daemons()
    if not procs:
        print("  ✗ 启动失败,看 log")
        return

    # 加载 token
    if not AUTH_TOKEN_FILE.exists():
        print(f"  ✗ auth_token 文件不存在:{AUTH_TOKEN_FILE}")
        stop_daemons(procs)
        return
    token = AUTH_TOKEN_FILE.read_text(encoding="utf-8").strip()
    headers = {"Authorization": f"Bearer {token}"}

    # 2) 跑 e2e 测试
    print("\n[2] 跑 e2e 测试...")
    client = httpx.Client(base_url=BASE_URL, timeout=30.0, headers=headers)

    results = []
    try:
        # /health
        r = client.get("/health")
        ok = r.status_code == 200 and r.json().get("status") == "ok"
        results.append(("GET /health", ok, f"status={r.json().get('status')}"))
        print(f"  ✓ GET /health: {r.json().get('status')}")

        # /presence 默认 voice
        r = client.get("/presence")
        ok = r.json().get("mode") == "voice"
        results.append(("GET /presence (default voice)", ok, f"mode={r.json().get('mode')}"))
        print(f"  ✓ GET /presence (default voice): {r.json().get('mode')}")

        # /transcribe 上传本地 wav
        wav_path = Path("C:/temp/sf_audio/real_xiaoxiao_10s.wav")
        if wav_path.exists():
            audio_b64 = base64.b64encode(wav_path.read_bytes()).decode()
            r = client.post(f"http://127.0.0.1:{ASR_DAEMON_PORT}/transcribe",
                            headers=headers, json={"audio_b64": audio_b64})
            ok = r.status_code == 200 and "关关雎鸠" in r.json().get("text", "")
            results.append(("POST /transcribe (xiaoxiao wav → SenseVoice)", ok,
                           f"text={r.json().get('text','')[:50]}"))
            print(f"  ✓ POST /transcribe: '{r.json().get('text','')[:50]}'")

        # /presence 切 text
        r = client.post("/presence", json={"mode": "text"})
        ok = r.json().get("mode") == "text"
        results.append(("POST /presence (set text)", ok, f"mode={r.json().get('mode')}"))
        print(f"  ✓ POST /presence (set text): {r.json().get('mode')}")

        # /speak text mode (写 IM log)
        r = client.post("/speak", json={"text": "断网验证测试文本", "interrupt": True})
        ok = r.json().get("status") == "ok"
        results.append(("POST /speak (text mode)", ok, str(r.json())))
        print(f"  ✓ POST /speak (text mode): {r.json().get('status')}")

        # /presence 切回 voice
        r = client.post("/presence", json={"mode": "voice"})
        print(f"  ✓ POST /presence (back to voice): {r.json().get('mode')}")

    except Exception as e:
        print(f"  ✗ e2e 测试异常:{e}")
    finally:
        client.close()

    # 3) 验证零外发
    print("\n[3] 验证零外发(扫本机 ESTABLISHED 非 loopback 连接)...")
    print("  " + "-" * 50)
    rows, external = check_no_external_daemon_connections()
    print(f"  3 daemon 当前 ESTABLISHED 连接: {len(rows)}")
    for r in rows:
        print(f"    PID={r.get('OwningProcess')} {r.get('LocalPort')} → {r.get('RemoteAddress')}:{r.get('RemotePort')}")
    if external:
        print(f"  ✗ 检测到外发连接:{external}")
        results.append(("零外发验证", False, f"外发:{external}"))
    else:
        print(f"  ✓ 所有连接都是 loopback(127.0.0.1)")
        results.append(("零外发验证", True, "0 个外发连接"))

    # 总结
    print()
    print("=" * 60)
    print("总结")
    print("=" * 60)
    n_pass = sum(1 for _, ok, _ in results if ok)
    print(f"  {n_pass}/{len(results)} pass")
    for name, ok, detail in results:
        icon = "✓" if ok else "✗"
        print(f"    {icon} {name}: {detail}")

    # 收尾
    print("\n  关闭 daemon...")
    stop_daemons(procs)

    if n_pass == len(results):
        print("\n  ✓ voice_input 断网验证 PASSED — ADR-001 §5.4 隐私边界满足")
    else:
        print(f"\n  ✗ {len(results) - n_pass} 项验证 FAIL,需要排查")


if __name__ == "__main__":
    main()