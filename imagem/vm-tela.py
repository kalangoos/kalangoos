#!/usr/bin/env python3
"""KalangoOS · mexe numa VM ligada por imagem/rodar-iso.sh pelo QMP (qmp.sock): captura a tela, clica e digita.

  vm-tela.py PASTA-DA-VM captura ARQUIVO.png
  vm-tela.py PASTA-DA-VM clicar X Y            (pixels da tela da VM, 1920x1080 por padrão: RES)
  vm-tela.py PASTA-DA-VM tecla ctrl+alt+f2     (nomes do QEMU: ret, tab, spc, esc, left, f11, ctrl-alt-f2 …)
  vm-tela.py PASTA-DA-VM texto "o que digitar" (só ASCII)

Serve para as capturas de tela e para percorrer o instalador sem mexer no computador de quem roda.
"""
import json
import os
import socket
import sys
import time

KEYS = {c: c for c in "abcdefghijklmnopqrstuvwxyz0123456789"}
KEYS.update({" ": "spc", "\n": "ret", "-": "minus", ".": "dot", ",": "comma", "/": "slash", "@": "shift-2", "_": "shift-minus", ":": "shift-semicolon"})


class Qmp:
    def __init__(self, pasta):
        self.s = socket.socket(socket.AF_UNIX)
        self.s.connect(os.path.join(pasta, "qmp.sock"))
        self.f = self.s.makefile("rw")
        self.f.readline()  # saudação
        self.cmd("qmp_capabilities")

    def cmd(self, nome, **args):
        self.f.write(json.dumps({"execute": nome, "arguments": args}) + "\n")
        self.f.flush()
        while True:
            r = json.loads(self.f.readline())
            if "return" in r or "error" in r:
                if "error" in r:
                    raise RuntimeError(r["error"])
                return r["return"]

    def captura(self, arquivo):
        self.cmd("screendump", filename=os.path.abspath(arquivo), format="png")

    def clicar(self, x, y, w=1920, h=1080):
        ev = [{"type": "abs", "data": {"axis": "x", "value": int(x * 32767 / (w - 1))}},
              {"type": "abs", "data": {"axis": "y", "value": int(y * 32767 / (h - 1))}}]
        self.cmd("input-send-event", events=ev)
        time.sleep(0.2)
        for down in (True, False):
            self.cmd("input-send-event", events=[{"type": "btn", "data": {"button": "left", "down": down}}])
            time.sleep(0.1)

    def tecla(self, combo):
        teclas = [{"type": "qcode", "data": k} for k in combo.replace("+", "-").split("-") if k]
        self.cmd("send-key", keys=teclas, **{"hold-time": 80})

    def texto(self, t):
        for ch in t:
            self.tecla(KEYS[ch] if ch.islower() or ch.isdigit() or ch in KEYS else ch)
            time.sleep(0.05)


if __name__ == "__main__":
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    q = Qmp(sys.argv[1])
    acao, args = sys.argv[2], sys.argv[3:]
    if acao == "captura":
        q.captura(args[0])
    elif acao == "clicar":
        q.clicar(int(args[0]), int(args[1]))
    elif acao == "tecla":
        q.tecla(args[0])
    elif acao == "texto":
        q.texto(args[0])
    else:
        sys.exit(__doc__)
