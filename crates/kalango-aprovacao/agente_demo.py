"""Agente de demonstração: pede ações ao kalangod, uma por vez, e mostra o que você decidiu na tela."""
import json, socket, sys, time
PEDIDOS = [
    ("pacotes.instalar", {"nomes": ["gimp", "inkscape"]}),
    ("servicos.acao", {"nome": "bluetooth.service", "acao": "parar"}),
    ("firewall.regra", {"acao": "permitir", "porta": "8080/tcp"}),
    ("usuarios.criar", {"nome": "convidado", "nome_completo": "Convidado"}),
    ("sistema.executar", {"programa": "apt-get", "argumentos": ["autoremove", "-y"]}),
    ("pacotes.remover", {"nomes": ["libreoffice-core"]}),
]
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); f = s.makefile("rw")
for i, (tool, args) in enumerate(PEDIDOS, 1):
    print(f"[agente] pedindo {tool} {json.dumps(args, ensure_ascii=False)} … (decida na tela)", flush=True)
    f.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": "tools/call", "params": {"name": tool, "arguments": args}}) + "\n"); f.flush()
    r = json.loads(f.readline())["result"]
    txt = r["content"][0]["text"]
    print(f"[agente]   → {'NEGADO: ' + txt if r['isError'] else 'APROVADO (simulado): ' + json.loads(txt).get('saida', json.loads(txt).get('comando', ''))}", flush=True)
    time.sleep(2)
print("[agente] fim da demonstração; feche a janela do niri", flush=True)
