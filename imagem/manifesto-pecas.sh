#!/bin/bash
# KalangoOS · manifesto das peças da ISO que a CI guarda em cache (niri, llama.cpp, Moli, Kalango Remote, Kalango Audio e a
# raiz de pacotes): quem constrói a peça grava o SHA-256 dos arquivos e de onde veio; a montagem da ISO confere os
# arquivos restaurados contra esse manifesto e o junta ao que a ISO publica.
#
#   imagem/manifesto-pecas.sh gravar <peça> <arquivo ou pasta>...   no run que CONSTRÓI a peça (vai dentro do cache)
#   imagem/manifesto-pecas.sh conferir <peça>                       em quem usa a peça (falha se algo mudou ou sumiu)
#
# O que isto garante e o que não: pega arquivo corrompido ou trocado depois de gravado e deixa registrado em que run e
# commit a peça foi construída. Não prova nada contra quem consegue gravar no cache E no manifesto ao mesmo tempo (quem
# escreve num cache escreve nos dois): a atestação da publicação cobre a montagem da ISO; a origem das peças em cache
# fica neste manifesto, que vai junto da ISO (PROVENIENCIA-PECAS.txt).
set -euo pipefail
cd "$(dirname "$0")/.."
PASTA=pecas-manifesto
cmd=${1:?uso: manifesto-pecas.sh gravar|conferir <peça> [arquivos]}
peca=${2:?falta o nome da peça}
arq=$PASTA/$peca.txt
case "$cmd" in
  gravar)
    shift 2
    [ $# -gt 0 ] || { echo "gravar: faltam os arquivos"; exit 2; }
    mkdir -p "$PASTA"
    {
      echo "# peça: $peca"
      echo "# construída em: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
      echo "# run: ${GITHUB_SERVER_URL:-local}/${GITHUB_REPOSITORY:-}/actions/runs/${GITHUB_RUN_ID:-0}"
      echo "# commit do repositório: ${GITHUB_SHA:-desconhecido}"
      find "$@" \( -type f -o -type l \) -print0 | sort -z | xargs -0 sha256sum
    } > "$arq"
    echo "manifesto de $peca: $(grep -vc '^#' "$arq") arquivos"
    ;;
  conferir)
    [ -s "$arq" ] || { echo "::error::falta o manifesto de $peca ($arq): a peça veio de um cache sem ele"; exit 1; }
    grep '^# ' "$arq"
    grep -v '^#' "$arq" | sha256sum --check --quiet || { echo "::error::arquivos de $peca diferem do manifesto"; exit 1; }
    echo "$peca: $(grep -vc '^#' "$arq") arquivos conferidos"
    ;;
  *) echo "uso: manifesto-pecas.sh gravar|conferir <peça>"; exit 2 ;;
esac
