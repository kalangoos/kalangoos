#!/usr/bin/env bash
# Monta a árvore pública do KalangoOS a partir de um commit do repositório de desenvolvimento:
#   scripts/publicacao/exportar.sh <commit> <pasta-de-destino> <substituicoes.tsv>
#
# 1. só os arquivos versionados no <commit> (git archive), sem os caminhos de excluir.txt;
# 2. troca dados pessoais por fictícios, pela tabela <substituicoes.tsv> ("original<TAB>substituto" por linha, a mais
#    longa primeiro). A tabela fica FORA do repositório: ela contém justamente o que não pode ser publicado;
# 3. confere: nenhum original da tabela sobrou, nenhum segredo (gitleaks, se estiver no PATH), nenhum arquivo acima
#    de 5 MB e nenhum caminho excluído.
# Não faz commit nem push: a pasta de destino recebe só os arquivos; o resto é com quem publica.
set -euo pipefail

[ $# -eq 3 ] || { echo "uso: $0 <commit> <pasta-de-destino> <substituicoes.tsv>" >&2; exit 2; }
commit=$1 dest=$2 table=$3
here=$(cd "$(dirname "$0")" && pwd)
repo=$(git -C "$here" rev-parse --show-toplevel)

[ -f "$table" ] || { echo "tabela de substituições não encontrada: $table" >&2; exit 2; }
case "$(realpath "$table")" in "$repo"/*) echo "a tabela de substituições não pode ficar dentro do repositório" >&2; exit 2 ;; esac
if [ -e "$dest" ] && [ -n "$(ls -A "$dest" 2>/dev/null)" ]; then echo "a pasta de destino precisa estar vazia: $dest" >&2; exit 2; fi
mkdir -p "$dest"

echo ">> arquivos do commit $(git -C "$repo" rev-parse --short "$commit")"
git -C "$repo" archive --format=tar "$commit" | tar -x -C "$dest"

echo ">> removendo os caminhos de excluir.txt"
mapfile -t patterns < <(grep -v '^\s*#' "$here/excluir.txt" | sed '/^\s*$/d')
removed=0
while IFS= read -r -d '' f; do
  rel=${f#"$dest"/}
  for p in "${patterns[@]}"; do
    # shellcheck disable=SC2053 # $p is a pattern on purpose
    if [[ "$rel" == $p ]]; then rm -f -- "$f"; removed=$((removed + 1)); break; fi
  done
done < <(find "$dest" \( -type f -o -type l \) -print0)
find "$dest" -type d -empty -delete
echo "   $removed arquivos removidos"

echo ">> trocando dados pessoais"
while IFS=$'\t' read -r from to; do
  [ -z "$from" ] && continue
  case "$from" in \#*) continue ;; esac
  while IFS= read -r -d '' f; do
    FROM=$from TO=$to perl -pi -e 's/\Q$ENV{FROM}\E/$ENV{TO}/g' "$f"
  done < <(grep -rlIZF -- "$from" "$dest" || true)
done < "$table"

echo ">> conferindo"
fail=0
while IFS=$'\t' read -r from _; do
  [ -z "$from" ] && continue
  case "$from" in \#*) continue ;; esac
  # short originals are only looked for in text: a few bytes can occur by chance inside compressed images
  if [ "${#from}" -ge 6 ]; then mode=-rlaF; else mode=-rlIF; fi
  left=$(grep "$mode" -- "$from" "$dest" | head -5 | tr '\n' ' ' || true)
  if [ -n "$left" ]; then echo "   SOBROU \"$from\" em: $left"; fail=1; fi
done < "$table"
big=$(find "$dest" -type f -size +5M)
[ -z "$big" ] || { echo "   arquivos acima de 5 MB: $big"; fail=1; }
while IFS= read -r -d '' f; do
  rel=${f#"$dest"/}
  for p in "${patterns[@]}"; do
    # shellcheck disable=SC2053
    if [[ "$rel" == $p ]]; then echo "   caminho excluído presente: $rel"; fail=1; fi
  done
done < <(find "$dest" -type f -print0)
if command -v gitleaks > /dev/null; then
  gitleaks dir --no-banner --redact --exit-code 1 "$dest" || fail=1
else
  echo "   (gitleaks fora do PATH: segredos não conferidos)"
fi
du -sh "$dest" | sed 's/^/   tamanho: /'
if [ "$fail" != 0 ]; then echo ">> FALHOU: corrija antes de publicar" >&2; exit 1; fi
echo ">> pronto: $dest"
