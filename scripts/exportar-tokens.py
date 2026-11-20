#!/usr/bin/env python3
"""KalangoOS · the colour tokens from the Design System (FigCraft's "KalangoOS · Cores" variables) to the code.

Writes design/tokens/tokens.json and the GTK sheets tokens-dark.css / tokens-light.css (@define-color k_<name>) in
design/tokens and in each app that draws the system (kalango-shell, kalango-entrada and kalango-aprovacao: these two
only use the dark one). The sheets are never edited by hand: change the variable in FigCraft and run this again.

    scripts/exportar-tokens.py [--porta 47800]

It talks to the FigCraft window that has the design open (figcraft-cli mcp --attach PORT).
"""
import argparse
import itertools
import json
import os
import subprocess
import sys

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
COLECAO = "KalangoOS · Cores"
MODOS = {"Escuro": "dark", "Claro": "light"}
DESTINOS = {
    "design/tokens": ("dark", "light"),
    "crates/kalango-shell/assets": ("dark", "light"),
    "crates/kalango-entrada/assets": ("dark",),
    "crates/kalango-aprovacao/assets": ("dark",),
    "crates/kalango-instalador/assets": ("dark", "light"),
}


class FigCraft:
    def __init__(self, porta):
        self.p = subprocess.Popen(["figcraft-cli", "mcp", "--attach", str(porta)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, text=True, bufsize=1)
        self.ids = itertools.count(1)
        self.rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "kalango-tokens", "version": "1"}})
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.p.stdin.flush()

    def rpc(self, metodo, params):
        i = next(self.ids)
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": metodo, "params": params}) + "\n")
        self.p.stdin.flush()
        while True:
            linha = self.p.stdout.readline()
            if not linha:
                sys.exit("o FigCraft não respondeu (o design está aberto, com a porta de controle?)")
            m = json.loads(linha)
            if m.get("id") == i:
                if "error" in m:
                    sys.exit(f"FigCraft: {m['error']}")
                return m["result"]

    def comando(self, cid, params=None):
        r = self.rpc("tools/call", {"name": "run_command", "arguments": {"command": cid, "params": params or {}}})
        texto = "\n".join(c.get("text", "") for c in r.get("content", []) if c.get("type") == "text")
        if r.get("isError"):
            sys.exit(f"FigCraft: {texto[:500]}")
        return json.loads(texto)


def hexa(c):
    return "#%02X%02X%02X" % tuple(round(c[k] * 255) for k in ("r", "g", "b"))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--porta", type=int, default=47800)
    a = ap.parse_args()
    fc = FigCraft(a.porta)
    dados = fc.comando("variable.list")
    # every collection (colours, fonts, layout, motion) goes to tokens.json: {collection: {name: {mode: value}}}
    todas = {}
    for col in dados["collections"]:
        modos = {m["id"]: m["name"] for m in col["modes"]}
        vs = {}
        for v in dados["variables"]:
            if v["collection"] != col["id"]:
                continue
            for mid, valor in v["values"].items():
                x = valor.get("value")
                if isinstance(x, dict) and "r" in x:
                    x = hexa(x)
                elif isinstance(x, dict):
                    continue
                vs.setdefault(v["name"], {})[modos.get(mid, mid)] = x
        todas[col["name"]] = dict(sorted(vs.items()))
    # the sheets: the colours of the colour collection
    tokens = {n: m for n, m in todas[COLECAO].items() if n.startswith("cor/")}
    folhas = {}
    for modo, nome in MODOS.items():
        rotulo = "Escuro" if nome == "dark" else "Claro"
        linhas = [f"/* KalangoOS · tokens de cor · modo {rotulo} · gerado a partir do Design System (não editar à mão) */"]
        linhas += [f"@define-color k_{n.split('/', 1)[1]} {c[modo]};" for n, c in tokens.items() if modo in c]
        folhas[nome] = "\n".join(linhas) + "\n"
    for pasta, quais in DESTINOS.items():
        d = os.path.join(RAIZ, pasta)
        if pasta == "design/tokens":
            with open(os.path.join(d, "tokens.json"), "w") as f:
                json.dump(dict(sorted(todas.items())), f, ensure_ascii=False, indent=1)
                f.write("\n")
        for q in quais:
            with open(os.path.join(d, f"tokens-{q}.css"), "w") as f:
                f.write(folhas[q])
    print(f"{sum(len(v) for v in todas.values())} variáveis em tokens.json; {len(tokens)} cores nas folhas de {', '.join(DESTINOS)}")


if __name__ == "__main__":
    main()
