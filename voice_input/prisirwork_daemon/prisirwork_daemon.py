# -*- coding: utf-8 -*-
"""PrisirWork 本地智能管家 — 接收端骨架(2026-08-19)

职责:接收输入法转发的内容,做智能处理,返回结果。
当前是骨架:监听本地 socket,收到 IME 转发 → 回显(占位),将来填智能逻辑。

协议(JSON Lines, 每条消息一行 JSON):
  请求:  {"type":"ime_forward","text":"...","intent":"polish|translate|reply|raw","context":{...}}
  响应:  {"type":"ime_result","ok":true,"text":"处理后的文本"}
"""
from __future__ import annotations
import json
import os
import socket
import socketserver
import sys
import threading

# pythonw 保护
if sys.stdout is None:
    sys.stdout = open(os.devnull, "w", encoding="utf-8", errors="replace")
if sys.stderr is None:
    sys.stderr = open(os.devnull, "w", encoding="utf-8", errors="replace")

HOST = "127.0.0.1"
PORT = 51234
LOG = os.path.join(os.path.dirname(os.path.abspath(__file__)), "prisirwork.log")


def log(msg):
    try:
        import time
        with open(LOG, "a", encoding="utf-8") as f:
            f.write(f"{time.strftime('%H:%M:%S')} {msg}\n")
    except Exception:
        pass


def process_text(text: str, intent: str, context: dict) -> str:
    """智能处理入口。

    优先识别超级命令(/前缀):/date /clip /bt /bth /calc。
    否则按 intent 走智能处理(骨架:回显,将来接云端模型)。
    """
    text = text.strip()
    if text.startswith("/"):
        return _run_command(text)
    # TODO(PrisirWork): 接云端模型做润色/翻译/回复;接屏幕上下文;跨应用编排。
    return f"[PrisirWork·{intent}] {text}"


# ---------------- 超级命令(本地纯逻辑,零外发) ----------------
def _run_command(text: str) -> str:
    """解析并执行 /命令。"""
    body = text[1:]  # 去掉 /
    parts = body.split(None, 1)
    name = parts[0].lower() if parts else ""
    arg = parts[1].strip() if len(parts) > 1 else ""

    handler = _COMMANDS.get(name)
    if not handler:
        return f"[未知命令] /{name}（可用: /date /clip /bt /bth /calc）"
    try:
        return handler(arg)
    except Exception as e:
        log(f"cmd /{name} failed: {e}")
        return f"[命令出错] /{name}: {e}"


def _cmd_date(arg: str) -> str:
    """当前日期时间。arg 可选格式,如 /date 或 /date %Y年%m月%d日。"""
    import datetime
    now = datetime.datetime.now()
    if arg:
        try:
            return now.strftime(arg)
        except Exception:
            pass
    return now.strftime("%Y-%m-%d %H:%M:%S %A")


def _cmd_clip(arg: str) -> str:
    """剪贴板历史(当前剪贴板内容)。"""
    try:
        import pyperclip
        clip = pyperclip.paste()
        if not clip:
            return "[剪贴板为空]"
        # 太长截断
        return clip if len(clip) <= 200 else clip[:200] + "…"
    except Exception as e:
        return f"[剪贴板读取失败] {e}"


def _cmd_bt(arg: str) -> str:
    """二进制转十进制:/bt 1010 -> 10"""
    if not arg:
        return "[用法] /bt 1010"
    try:
        return str(int(arg, 2))
    except ValueError:
        return f"[无效二进制] {arg}"


def _cmd_bth(arg: str) -> str:
    """十进制转十六进制:/bth 255 -> 0xff"""
    if not arg:
        return "[用法] /bth 255"
    try:
        return hex(int(arg))
    except ValueError:
        return f"[无效十进制] {arg}"


def _cmd_calc(arg: str) -> str:
    """安全计算表达式:/calc 1+2*3 -> 7。只允许数字和运算符。"""
    if not arg:
        return "[用法] /calc 1+2*3"
    # 白名单字符,防注入
    allowed = set("0123456789+-*/().,% ")
    if not all(c in allowed for c in arg):
        return "[含非法字符] 只允许数字和 +-*/().,%"
    try:
        # 用 ast 安全求值而非 eval
        import ast, operator as op
        ops = {ast.Add: op.add, ast.Sub: op.sub, ast.Mult: op.mul,
               ast.Div: op.truediv, ast.Mod: op.mod, ast.USub: op.neg,
               ast.UAdd: op.pos}
        def ev(node):
            if isinstance(node, ast.Num):
                return node.n
            if isinstance(node, ast.Constant):
                return node.value
            if isinstance(node, ast.BinOp):
                return ops[type(node.op)](ev(node.left), ev(node.right))
            if isinstance(node, ast.UnaryOp):
                return ops[type(node.op)](ev(node.operand))
            raise ValueError("不支持的表达式")
        result = ev(ast.parse(arg, mode="eval").body)
        # 整数结果去小数点
        if isinstance(result, float) and result.is_integer():
            result = int(result)
        return str(result)
    except Exception as e:
        return f"[计算错误] {e}"


_COMMANDS = {
    "date": _cmd_date,
    "clip": _cmd_clip,
    "bt": _cmd_bt,
    "bth": _cmd_bth,
    "calc": _cmd_calc,
}


class _Handler(socketserver.StreamRequestHandler):
    def handle(self):
        line = self.rfile.readline()
        if not line:
            return
        try:
            req = json.loads(line.decode("utf-8", errors="replace"))
        except Exception as e:
            log(f"bad request: {e}")
            self._send({"type": "ime_result", "ok": False, "text": ""})
            return
        text = req.get("text", "")
        intent = req.get("intent", "raw")
        context = req.get("context", {})
        log(f"recv: intent={intent} text={text!r} ctx={context}")
        out = process_text(text, intent, context)
        self._send({"type": "ime_result", "ok": True, "text": out})

    def _send(self, obj):
        self.wfile.write((json.dumps(obj, ensure_ascii=False) + "\n").encode("utf-8"))


class _Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


def main():
    log(f"=== PrisirWork daemon start {HOST}:{PORT} ===")
    with _Server((HOST, PORT), _Handler) as srv:
        log("listening")
        srv.serve_forever()


if __name__ == "__main__":
    main()
