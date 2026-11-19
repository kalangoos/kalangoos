#!/usr/bin/python3
"""KalangoOS · what the Central de Apps reads, for tests (nothing of the person's, nothing from the network):
usage: loja-teste.py DIR — writes DIR/appstream (a Flatpak remote's catalog, laid out as flatpak's, with icons),
DIR/dados (the essentials' shelf and a models catalog whose test model is a local file, DIR/mini.gguf).
The shell reads them with KALANGO_LOJA_APPSTREAM=DIR/appstream and KALANGO_LOJA_DADOS=DIR/dados; installs go to the
flatpak stand-in (flatpak-teste, which already has org.exemplo.Fotos)."""
import json
import os
import struct
import sys
import zlib

d = sys.argv[1]
active = os.path.join(d, "appstream", "flathub", "x86_64", "active")
icons = os.path.join(active, "icons", "64x64")
os.makedirs(icons, exist_ok=True)
os.makedirs(os.path.join(d, "dados"), exist_ok=True)


def png(p, cor, w=64, h=64):
    ch = lambda t, b: struct.pack('>I', len(b)) + t + b + struct.pack('>I', zlib.crc32(t + b) & 0xffffffff)
    raw = b''.join(b'\0' + bytes(cor) * w for _ in range(h))
    open(p, 'wb').write(b'\x89PNG\r\n\x1a\n' + ch(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)) + ch(b'IDAT', zlib.compress(raw)) + ch(b'IEND', b''))


apps = [
    ("org.exemplo.Fotos", "Fotos", "Suas fotos, organizadas", "Graphics;Photography", (90, 160, 200)),
    ("org.exemplo.Notas", "Notas", "Anotações rápidas", "Office;TextEditor", (230, 200, 120)),
    ("org.exemplo.Jogo", "Jogo da Velha", "Um jogo para dois", "Game;BoardGame", (200, 120, 160)),
    ("org.exemplo.Codigo", "Código", "Um editor de código", "Development;IDE", (140, 200, 180)),
    ("org.exemplo.Mapas", "Mapas", "Mapas e rotas", "Utility;Maps", (120, 180, 120)),
]
comps = []
for i, (aid, name, summary, cats, cor) in enumerate(apps):
    png(os.path.join(icons, f"{aid}.png"), cor)
    c = "".join(f"<category>{x}</category>" for x in cats.split(";"))
    comps.append(f"""  <component type="desktop-application">
    <id>{aid}</id>
    <name>{name}</name>
    <summary>{summary}</summary>
    <categories>{c}</categories>
    <keywords><keyword>{name.lower()}</keyword></keywords>
    <icon type="cached" height="64" width="64">{aid}.png</icon>
    <bundle type="flatpak">app/{aid}/x86_64/stable</bundle>
  </component>""")
open(os.path.join(active, "appstream.xml"), "w").write('<?xml version="1.0" encoding="UTF-8"?>\n<components version="0.8" origin="flathub">\n' + "\n".join(comps) + "\n</components>\n")
shelf = [{"id": "org.exemplo.Fotos", "categoria": "criacao"}, {"id": "org.exemplo.Notas", "categoria": "produtividade"},
         {"id": "org.exemplo.Jogo", "categoria": "jogos"}, {"id": "org.exemplo.Codigo", "categoria": "desenvolvimento"}]
json.dump(shelf, open(os.path.join(d, "dados", "essenciais.json"), "w"), ensure_ascii=False)
mini = os.path.join(d, "mini.gguf")
open(mini, "wb").write(b"GGUF" + bytes(3 * 1024 * 1024 - 4))
models = [
    {"id": "lfm3-1.7b", "nome": "LFM3 1.7B", "descricao": "conversa, ferramentas e código", "tipo": "texto",
     "arquivos": [{"url": "file:///nao/baixar", "nome": "LFM3-1.7B-Q4_K_M.gguf", "bytes": 1099627808}]},
    {"id": "teste-mini", "nome": "Teste Mini", "descricao": "modelo de teste (local)", "tipo": "texto",
     "arquivos": [{"url": "file://" + mini, "nome": "teste-mini.gguf", "bytes": os.path.getsize(mini)}]},
    {"id": "teste-grande", "nome": "Teste Grande", "descricao": "maior que qualquer GPU", "tipo": "texto",
     "arquivos": [{"url": "file:///nao/baixar", "nome": "teste-grande.gguf", "bytes": 900_000_000_000}]},
]
json.dump(models, open(os.path.join(d, "dados", "modelos.json"), "w"), ensure_ascii=False)
print(d)
