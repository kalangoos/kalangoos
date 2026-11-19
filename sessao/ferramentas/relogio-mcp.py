#!/usr/bin/python3
"""A small MCP connector (stdio) for tests and demos: the time in a city ("relogio"). Read only, no network:
it uses the system's time zone database. It runs isolated through `kalangod conector rodar`."""
import json, sys, datetime, unicodedata
from zoneinfo import ZoneInfo

CIDADES = {"toquio": "Asia/Tokyo", "tokyo": "Asia/Tokyo", "londres": "Europe/London", "london": "Europe/London",
           "nova york": "America/New_York", "new york": "America/New_York", "lisboa": "Europe/Lisbon", "paris": "Europe/Paris",
           "sydney": "Australia/Sydney", "sao paulo": "America/Sao_Paulo", "recife": "America/Recife", "berlim": "Europe/Berlin"}
TOOLS = [{"name": "hora_em", "description": "Hora atual numa cidade (Tóquio, Londres, Nova York, Lisboa, Paris, Sydney, São Paulo, Recife, Berlim)",
          "annotations": {"readOnlyHint": True},
          "inputSchema": {"type": "object", "properties": {"cidade": {"type": "string"}}, "required": ["cidade"]}}]

def simples(s):
    return "".join(c for c in unicodedata.normalize("NFD", s.lower()) if unicodedata.category(c) != "Mn").strip()

def hora_em(cidade):
    tz = CIDADES.get(simples(cidade))
    if not tz:
        raise ValueError("cidade desconhecida: " + cidade)
    agora = datetime.datetime.now(ZoneInfo(tz))
    return {"cidade": cidade, "fuso": tz, "hora": agora.strftime("%H:%M"), "data": agora.strftime("%d/%m/%Y")}

for linha in sys.stdin:
    try:
        m = json.loads(linha)
    except ValueError:
        continue
    if "id" not in m:
        continue
    met, r = m.get("method"), None
    try:
        if met == "initialize":
            r = {"protocolVersion": m["params"].get("protocolVersion", "2025-06-18"), "capabilities": {"tools": {}}, "serverInfo": {"name": "relogio", "version": "1"}}
        elif met == "tools/list":
            r = {"tools": TOOLS}
        elif met == "tools/call" and m["params"]["name"] == "hora_em":
            v = hora_em(m["params"].get("arguments", {}).get("cidade", ""))
            r = {"content": [{"type": "text", "text": json.dumps(v, ensure_ascii=False)}], "structuredContent": v, "isError": False}
        else:
            r = {"content": [{"type": "text", "text": "ferramenta desconhecida"}], "isError": True}
        out = {"jsonrpc": "2.0", "id": m["id"], "result": r}
    except Exception as e:
        out = {"jsonrpc": "2.0", "id": m["id"], "result": {"content": [{"type": "text", "text": str(e)}], "isError": True}}
    sys.stdout.write(json.dumps(out, ensure_ascii=False) + "\n")
    sys.stdout.flush()
