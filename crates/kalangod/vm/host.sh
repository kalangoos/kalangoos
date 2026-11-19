#!/bin/bash
# KalangoOS · valida no host, como root e com systemd, o que a VM não tem: NetworkManager, udisks, SMART e as
# tarefas via systemd-run. Só faz mudanças descartáveis (uma conexão dummy, uma imagem loop) e desfaz tudo no fim.
#   sudo vm/host.sh
set -u
cd "$(dirname "$0")/.."
[ "$(id -u)" = 0 ] || { echo "rode com sudo"; exit 1; }
K=$(realpath ../../target/release/kalangod)
T=$(mktemp -d /tmp/kalango-host-XXXX); chmod 711 "$T"  # o estado dentro fica 0700 (o kalangod garante)
cp dist/kalango-agentes.slice /run/systemd/system/ && systemctl daemon-reload && systemctl start kalango-agentes.slice
sed -e "s#^socket = .*#socket = \"$T/mcp.sock\"#; s#^state_dir = .*#state_dir = \"$T/estado\"#; s/^approval_requires_tty = true/approval_requires_tty = false/" dist/kalangod.toml > "$T/kalangod.toml"
"$K" servir --config "$T/kalangod.toml" > "$T/kalangod.log" 2>&1 &
KP=$!
for i in $(seq 100); do [ -S "$T/mcp.sock" ] && break; sleep 0.1; done
stat -c "estado: %a %U" "$T/estado"
KALANGOD="$K" SOCK="$T/mcp.sock" python3 -I vm/host.py; rc=$?
kill $KP; wait $KP 2>/dev/null
echo "--- kalangod:"; head -3 "$T/kalangod.log"
rm -rf "$T"
systemctl stop kalango-agentes.slice; rm -f /run/systemd/system/kalango-agentes.slice; systemctl daemon-reload
exit $rc
