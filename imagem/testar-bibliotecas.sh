#!/bin/bash
# Testa imagem/bibliotecas-chamadas.sh: uma biblioteca do sistema que o programa abre por nome conta; a que ele traz
# embutida (libkalango_*.so, como a do VirtualDesk) não conta. Sem a regra, a construção da ISO com o VirtualDesk
# parava em "bibliotecas que faltam na imagem: libkalango_netguard.so".
set -euo pipefail
cd "$(dirname "$0")/.."
. imagem/bibliotecas-chamadas.sh
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
printf 'int main(void){return 0;}\nconst char *a = "libwayland-server.so.0";\nconst char *b = "libkalango_netguard.so";\n' > "$T/p.c"
cc -o "$T/p" "$T/p.c"
out=$(bibliotecas_chamadas "$T/p")
echo "$out" | grep -qx 'libwayland-server.so.0' || { echo "FALHOU: a biblioteca aberta por nome sumiu: $out"; exit 1; }
! echo "$out" | grep -q 'libkalango_netguard' || { echo "FALHOU: a biblioteca embutida contou como do sistema: $out"; exit 1; }
echo "ok: bibliotecas-chamadas"
