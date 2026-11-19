"""Servidor MCP remoto de teste (HTTP) com OAuth 2.1: metadados, registro dinâmico, PKCE S256, resource, tokens que
vencem e renovação. Imprime a porta na primeira linha. Uso: python3 servidor_remoto.py [segundos_de_validade]"""
import base64, hashlib, json, secrets, sys, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlencode, urlparse

VALIDADE = int(sys.argv[1]) if len(sys.argv) > 1 else 2
E = {"clientes": {}, "codigos": {}, "acessos": {}, "renovacoes": set(), "contagem": {"autorizacoes": 0, "renovacoes": 0, "chamadas": 0}, "sessao": None}
L = threading.Lock()

class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def base(self): return f"http://127.0.0.1:{self.server.server_port}"
    def manda(self, code, corpo=b"", tipo="application/json", extra=None):
        self.send_response(code)
        for k, v in (extra or {}).items(): self.send_header(k, v)
        if corpo: self.send_header("Content-Type", tipo)
        self.send_header("Content-Length", str(len(corpo))); self.end_headers(); self.wfile.write(corpo)
    def js(self, code, v, extra=None): self.manda(code, json.dumps(v).encode(), extra=extra)
    def corpo(self): return self.rfile.read(int(self.headers.get("Content-Length", 0)))

    def do_GET(self):
        u = urlparse(self.path); q = {k: v[0] for k, v in parse_qs(u.query).items()}
        if u.path == "/.well-known/oauth-protected-resource/mcp":
            return self.js(200, {"resource": self.base() + "/mcp", "authorization_servers": [self.base()]})
        if u.path == "/.well-known/oauth-authorization-server":
            b = self.base()
            return self.js(200, {"issuer": b, "authorization_endpoint": b + "/autorizar", "token_endpoint": b + "/token", "registration_endpoint": b + "/registrar",
                                 "code_challenge_methods_supported": ["S256"], "grant_types_supported": ["authorization_code", "refresh_token"]})
        if u.path == "/autorizar":
            c = E["clientes"].get(q.get("client_id"))
            ok = c and q.get("redirect_uri") in c and q.get("code_challenge_method") == "S256" and q.get("resource") == self.base() + "/mcp" and q.get("state")
            if not ok: return self.js(400, {"error": "invalid_request", "q": q})
            code = secrets.token_urlsafe(16)
            E["codigos"][code] = (q["client_id"], q["code_challenge"], q["redirect_uri"], q["resource"])
            E["contagem"]["autorizacoes"] += 1
            return self.manda(302, extra={"Location": q["redirect_uri"] + "?" + urlencode({"code": code, "state": q["state"]})})
        self.manda(404)

    def emite(self, renovacao=None):
        a = secrets.token_urlsafe(16); r = renovacao or secrets.token_urlsafe(16)
        E["acessos"][a] = time.time() + VALIDADE; E["renovacoes"].add(r)
        return {"access_token": a, "token_type": "Bearer", "expires_in": VALIDADE, "refresh_token": r}

    def do_POST(self):
        u = urlparse(self.path); b = self.corpo()
        with L:
            if u.path == "/registrar":
                d = json.loads(b); cid = "cliente-" + secrets.token_hex(4); E["clientes"][cid] = d["redirect_uris"]
                return self.js(201, {"client_id": cid})
            if u.path == "/token":
                f = {k: v[0] for k, v in parse_qs(b.decode()).items()}
                if f.get("resource") != self.base() + "/mcp": return self.js(400, {"error": "invalid_target"})
                if f.get("grant_type") == "authorization_code":
                    c = E["codigos"].pop(f.get("code"), None)
                    desafio = base64.urlsafe_b64encode(hashlib.sha256(f.get("code_verifier", "").encode()).digest()).rstrip(b"=").decode()
                    if not c or c[0] != f.get("client_id") or c[2] != f.get("redirect_uri") or c[1] != desafio:
                        return self.js(400, {"error": "invalid_grant"})
                    return self.js(200, self.emite())
                if f.get("grant_type") == "refresh_token":
                    if f.get("refresh_token") not in E["renovacoes"]: return self.js(400, {"error": "invalid_grant"})
                    E["renovacoes"].discard(f["refresh_token"]); E["contagem"]["renovacoes"] += 1
                    return self.js(200, self.emite())
                return self.js(400, {"error": "unsupported_grant_type"})
            if u.path == "/revogar":
                E["acessos"].clear(); E["renovacoes"].clear(); return self.js(200, {})
            if u.path != "/mcp": return self.manda(404)
            auth = self.headers.get("Authorization", "")
            if not auth.startswith("Bearer ") or E["acessos"].get(auth[7:], 0) < time.time():
                meta = self.base() + "/.well-known/oauth-protected-resource/mcp"
                return self.js(401, {"error": "invalid_token"}, {"WWW-Authenticate": f'Bearer resource_metadata="{meta}"'})
            m = json.loads(b)
            if "id" not in m: return self.manda(202)
            met = m["method"]
            if met == "initialize":
                E["sessao"] = secrets.token_hex(8)
                r = {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "remoto-teste", "version": "1"}}
                return self.js(200, {"jsonrpc": "2.0", "id": m["id"], "result": r}, {"Mcp-Session-Id": E["sessao"]})
            if self.headers.get("Mcp-Session-Id") != E["sessao"] or self.headers.get("MCP-Protocol-Version") != "2025-06-18":
                return self.js(400, {"error": "sessão ou versão do protocolo ausente"})
            if met == "tools/list":
                t = [{"name": "eco", "description": "repete", "annotations": {"readOnlyHint": True}, "inputSchema": {"type": "object"}},
                     {"name": "contadores", "description": "contagens do servidor", "annotations": {"readOnlyHint": True}, "inputSchema": {"type": "object"}},
                     {"name": "perigosa", "description": "apaga o repositório", "inputSchema": {"type": "object"}}]
                return self.js(200, {"jsonrpc": "2.0", "id": m["id"], "result": {"tools": t}})
            if met == "tools/call":
                E["contagem"]["chamadas"] += 1
                n, a = m["params"]["name"], m["params"].get("arguments", {})
                txt = a.get("texto", "") if n == "eco" else json.dumps(E["contagem"]) if n == "contadores" else "não devia chegar aqui"
                # resposta em SSE: uma notificação de progresso e o resultado
                ev = [{"jsonrpc": "2.0", "method": "notifications/progress", "params": {"progress": 1}},
                      {"jsonrpc": "2.0", "id": m["id"], "result": {"content": [{"type": "text", "text": txt}], "isError": False}}]
                return self.manda(200, "".join(f"event: message\ndata: {json.dumps(e)}\n\n" for e in ev).encode(), "text/event-stream")
            return self.js(200, {"jsonrpc": "2.0", "id": m["id"], "error": {"code": -32601, "message": met}})

    def do_DELETE(self): self.manda(200)

s = ThreadingHTTPServer(("127.0.0.1", 0), H)
print(s.server_port, flush=True)
s.serve_forever()
