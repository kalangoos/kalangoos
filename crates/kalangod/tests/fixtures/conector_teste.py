"""Conector MCP de teste (stdio). Cada ferramenta tenta algo que o isolamento deve permitir ou impedir."""
import ctypes, json, os, socket, subprocess, sys

FERRAMENTAS = [
    {"name": "ler", "description": "lê um arquivo", "annotations": {"readOnlyHint": True}, "inputSchema": {"type": "object"}},
    {"name": "escrever", "description": "escreve um arquivo", "annotations": {"destructiveHint": False}, "inputSchema": {"type": "object"}},
    {"name": "conectar", "description": "abre conexão TCP local", "inputSchema": {"type": "object"}},
    {"name": "ambiente", "description": "variáveis de ambiente", "annotations": {"readOnlyHint": True}, "inputSchema": {"type": "object"}},
    {"name": "cgroup", "description": "cgroup do processo", "annotations": {"readOnlyHint": True}, "inputSchema": {"type": "object"}},
    {"name": "ptrace", "description": "tenta ptrace", "inputSchema": {"type": "object"}},
    {"name": "executar", "description": "executa um programa", "inputSchema": {"type": "object"}},
    {"name": "memoria", "description": "aloca memória", "inputSchema": {"type": "object"}},
    {"name": "valor", "description": "valor de uma variável", "annotations": {"readOnlyHint": True}, "inputSchema": {"type": "object"}},
    {"name": "perigosa", "description": "apaga tudo", "annotations": {"destructiveHint": True}, "inputSchema": {"type": "object"}},
]

def chamar(nome, a):
    if nome == "ler":
        return open(os.path.expanduser(a["caminho"])).read()
    if nome == "escrever":
        open(os.path.expanduser(a["caminho"]), "w").write(a["conteudo"]); return "ok"
    if nome == "conectar":
        s = socket.create_connection(("127.0.0.1", a["porta"]), timeout=3); s.close(); return "ok"
    if nome == "ambiente":
        return " ".join(sorted(os.environ))
    if nome == "cgroup":
        return open("/proc/self/cgroup").read()
    if nome == "ptrace":
        libc = ctypes.CDLL(None, use_errno=True)
        r = libc.ptrace(0, 0, 0, 0)  # PTRACE_TRACEME: normalmente permitido
        return f"{r} {ctypes.get_errno()}"
    if nome == "executar":
        return subprocess.run(a["programa"], capture_output=True, text=True).stdout
    if nome == "memoria":
        x = bytearray(a["mb"] * 1024 * 1024); return str(len(x))
    if nome == "valor":
        return os.environ.get(a["nome"], "<ausente>")
    if nome == "perigosa":
        return "não devia chegar aqui"
    raise ValueError(nome)

for linha in sys.stdin:
    m = json.loads(linha)
    if "id" not in m:
        continue
    met = m["method"]
    if met == "initialize":
        r = {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "conector-teste", "version": "1"}}
    elif met == "tools/list":
        r = {"tools": FERRAMENTAS}
    elif met == "tools/call":
        try:
            r = {"content": [{"type": "text", "text": str(chamar(m["params"]["name"], m["params"].get("arguments", {})))}], "isError": False}
        except Exception as e:
            r = {"content": [{"type": "text", "text": f"{type(e).__name__}: {e}"}], "isError": True}
    else:
        print(json.dumps({"jsonrpc": "2.0", "id": m["id"], "error": {"code": -32601, "message": met}}), flush=True); continue
    print(json.dumps({"jsonrpc": "2.0", "id": m["id"], "result": r}), flush=True)
