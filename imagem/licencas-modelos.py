#!/usr/bin/env python3
"""KalangoOS · o aviso de licenças dos modelos de IA que uma ISO traz (chamado por imagem/construir.sh).

    licencas-modelos.py <catalogo modelos.json> <licencas-modelos.json> <pasta dos modelos na camada> <saida LICENCAS.txt>

Lê os arquivos da pasta, acha de que modelo do catálogo cada um é, confere o tamanho (e a soma SHA-256, a menos que
KALANGO_IMAGEM_VERIFICAR_MODELOS=0) e escreve o texto que acompanha a ISO em /usr/share/doc/kalango/modelos/LICENCAS.txt:
por modelo, a licença, o fornecedor, o aviso que a licença manda manter e, no fim, o texto de cada licença.
Sem nenhum modelo, não escreve nada. Sai com 1 (a ISO não é gerada) quando:
  - um arquivo da pasta não é de nenhum modelo do catálogo (ou tem o tamanho ou a soma errados);
  - um modelo cuja licença o texto não está em sessao/dist/share/doc/kalango/modelos/ (ainda não decidido).
Os modelos "somente_download" (LFM3 e d1: a decisão é baixar, com o aceite da licença, nunca embutir) contam como não aprovados.
Imprime na saída padrão "APROVADO" quando todos os modelos têm "aprovado": true em licencas-modelos.json (decisão do
mantenedor, docs/ISO-CONTEUDO.md) e "NAO-APROVADO <ids>" quando não (só ISO local). Com KALANGO_IMAGEM_PUBLICAVEL=1 qualquer arquivo na pasta é erro.
"""
import hashlib
import json
import os
import sys


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 22), b""):
            h.update(b)
    return h.hexdigest()


def main():
    catalogo, dados, pasta, saida = sys.argv[1:5]
    publicavel = os.environ.get("KALANGO_IMAGEM_PUBLICAVEL", "0") == "1"
    # (a ISO publicável confere sempre a soma; só uma ISO local pode pular)
    verificar = publicavel or os.environ.get("KALANGO_IMAGEM_VERIFICAR_MODELOS", "1") != "0"
    modelos = json.load(open(catalogo))
    d = json.load(open(dados))
    # (todo arquivo da pasta, não só os .gguf: o construir.sh copia o que a lista pedir)
    aqui = sorted(f for f in os.listdir(pasta) if os.path.isfile(os.path.join(pasta, f))) if os.path.isdir(pasta) else []
    if not aqui:
        return 0
    if publicavel:
        print("KALANGO_IMAGEM_PUBLICAVEL=1: uma ISO publicável não leva modelos de IA, e há na imagem: " + ", ".join(aqui), file=sys.stderr)
        return 1
    por_arquivo = {a["nome"]: (m, a) for m in modelos for a in m["arquivos"]}
    achados = {}
    erros = []
    for f in aqui:
        if f not in por_arquivo:
            erros.append(f"{f}: não é de nenhum modelo do catálogo (sessao/dist/share/kalango/loja/modelos.json): sem licença conhecida")
            continue
        m, a = por_arquivo[f]
        caminho = os.path.join(pasta, f)
        if os.path.getsize(caminho) != a["bytes"]:
            erros.append(f"{f}: o tamanho não é o do catálogo ({os.path.getsize(caminho)} em vez de {a['bytes']})")
        elif verificar and sha256(caminho) != a["sha256"]:
            erros.append(f"{f}: a soma SHA-256 não é a do catálogo")
        achados.setdefault(m["id"], (m, []))[1].append(f)
    # (the full texts ship in every ISO, for the licence acceptance: sessao/dist/share/doc/kalango/modelos)
    textos_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "sessao", "dist", "share", "doc", "kalango", "modelos")
    textos = {}
    fam_dos_ids = {}
    for mid, (m, arqs) in achados.items():
        fam = d["familias"].get(d["modelos"].get(mid, ""))
        if not fam:
            erros.append(f"{mid}: sem licença em imagem/licencas-modelos.json")
            continue
        fam_dos_ids[mid] = fam
        if not fam["texto"] or not os.path.isfile(os.path.join(textos_dir, fam["texto"])):
            erros.append(f"{mid}: o texto da licença ({fam['licenca']}) não está em sessao/dist/share/doc/kalango/modelos/: decisão do mantenedor pendente (docs/ISO-CONTEUDO.md)")
        else:
            textos[fam["licenca"]] = open(os.path.join(textos_dir, fam["texto"])).read()
    if erros:
        print("\n".join(erros), file=sys.stderr)
        return 1
    L = []
    L.append("KalangoOS · modelos de IA nesta ISO / AI models in this ISO")
    L.append("=" * 62)
    L.append("")
    L.append("Esta ISO traz os modelos abaixo em /usr/share/kalango/modelos. Cada um é de outro autor e tem a sua licença;")
    L.append("elas não são a licença do KalangoOS (GPL-3.0-or-later). O texto de cada licença está no fim deste arquivo.")
    L.append("This ISO carries the models below in /usr/share/kalango/modelos. Each has its author's own licence, which is")
    L.append("not the KalangoOS licence (GPL-3.0-or-later). The text of each licence is at the end of this file.")
    L.append("")
    for mid in sorted(achados):
        m, arqs = achados[mid]
        fam = fam_dos_ids[mid]
        L.append(f"* {m['nome']} ({mid})")
        L.append(f"    arquivos / files: {', '.join(arqs)}")
        L.append(f"    licença / licence: {fam['licenca']} ({fam['spdx']})")
        L.append(f"    fornecedor / provider: {fam['fornecedor']}")
        L.append(f"    texto / text: {fam['url']}")
        L.append(f"    aviso / notice: {fam['aviso']}")
        L.append("")
    for nome, t in sorted(textos.items()):
        L.append("-" * 62)
        L.append(nome)
        L.append("-" * 62)
        L.append(t.rstrip("\n"))
        L.append("")
    os.makedirs(os.path.dirname(saida), exist_ok=True)
    open(saida, "w").write("\n".join(L))
    nao = sorted(mid for mid in achados if not fam_dos_ids[mid]["aprovado"] or fam_dos_ids[mid].get("somente_download"))
    print("APROVADO" if not nao else "NAO-APROVADO " + " ".join(nao))
    return 0


sys.exit(main())
