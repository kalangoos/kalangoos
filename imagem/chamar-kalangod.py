#!/usr/bin/env python3
"""KalangoOS · chama uma ferramenta do kalangod pelo soquete MCP e aprova o pedido (só para testes numa VM).

    sudo KALANGOD=/caminho/kalangod KALANGOD_SOCKET=/tmp/kd/mcp.sock chamar-kalangod.py FERRAMENTA ['{"arg": 1}']

Enquanto a chamada espera a aprovação, aprova pelo próprio kalangod (`kalangod aprovar N`), como a pessoa faria na
tela de aprovação. Imprime o texto da resposta; sai com 1 se a ferramenta deu erro.
"""
import json, os, socket, subprocess, sys, threading, time

K = os.environ["KALANGOD"]
s = socket.socket(socket.AF_UNIX)
s.connect(os.environ["KALANGOD_SOCKET"])
f = s.makefile("rw")
n = 0


def req(method, params):
    global n
    n += 1
    f.write(json.dumps({"jsonrpc": "2.0", "id": n, "method": method, "params": params}) + "\n")
    f.flush()
    while True:
        v = json.loads(f.readline())
        if v.get("id") == n:
            return v


def approve():
    for _ in range(120):
        out = subprocess.run([K, "pendentes"], capture_output=True, text=True).stdout
        if out.startswith("#"):
            print(subprocess.run([K, "aprovar", out.split()[0][1:]], capture_output=True, text=True).stdout.strip(), file=sys.stderr)
            return
        time.sleep(1)


req("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "teste", "version": "1"}})
threading.Thread(target=approve, daemon=True).start()
r = req("tools/call", {"name": sys.argv[1], "arguments": json.loads(sys.argv[2] if len(sys.argv) > 2 else "{}")})
res = r.get("result", {})
print(res.get("content", [{}])[0].get("text", r.get("error")))
sys.exit(1 if res.get("isError") or "error" in r else 0)
