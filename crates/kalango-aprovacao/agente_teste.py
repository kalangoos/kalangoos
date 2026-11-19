"""Agente de teste: pede uma ferramenta ao kalangod e imprime a resposta (usado por validar.sh)."""
import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); f = s.makefile("rw")
f.write(json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": sys.argv[2], "arguments": json.loads(sys.argv[3])}}) + "\n"); f.flush()
r = json.loads(f.readline())["result"]
print(json.dumps({"isError": r["isError"], "texto": r["content"][0]["text"][:300]}, ensure_ascii=False, indent=1))
