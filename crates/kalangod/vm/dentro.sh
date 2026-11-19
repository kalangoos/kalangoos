#!/bin/bash
# Roda dentro da VM, como root, sobre a camada descartável. Sobe o kalangod como serviço do sistema e testa
# como um agente (uid 1000) e como root.
set -u
D=$(dirname "$(realpath "$0")")
K=$(realpath "$D/../../../target/release/kalangod")
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin LC_ALL=C
mkdir -p /etc/kalango /var/lib/kalango
sed -e 's/^approval_requires_tty = true/approval_requires_tty = false/' "$D/../dist/kalangod.toml" > /etc/kalango/kalangod.toml
# os testes de limites usam valores pequenos
sed -i 's/^memoria_mb = .*/memoria_mb = 512/; s/^tempo_s = .*/tempo_s = 60/' /etc/kalango/kalangod.toml
modprobe loop 2>/dev/null; modprobe btrfs 2>/dev/null
# um cofre de arquivo do usuário (a guarda precisa protegê-lo de agentes)
mkdir -p /home/home/.local/share/kalango && echo '{"x":"segredo"}' > /home/home/.local/share/kalango/cofre.json && chown -R 1000:1000 /home/home/.local/share/kalango && chmod 600 /home/home/.local/share/kalango/cofre.json
"$K" servir --config /etc/kalango/kalangod.toml > /tmp/kalangod.log 2>&1 &
for i in $(seq 100); do [ -S /run/kalango/mcp.sock ] && break; sleep 0.1; done
sleep 1
echo "--- kalangod:"; head -5 /tmp/kalangod.log
# referência: fora do cgroup dos agentes, root lê /etc/shadow normalmente
head -c 4 /etc/shadow >/dev/null && echo "root fora dos agentes lê /etc/shadow: sim"
KALANGOD="$K" python3 -I "$D/testes.py"
rc=$?
echo "--- últimas linhas do kalangod:"; tail -5 /tmp/kalangod.log
exit $rc
