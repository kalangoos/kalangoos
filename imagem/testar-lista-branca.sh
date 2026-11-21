#!/bin/bash
# KalangoOS · the agents' allowlist with kalangod as root, as the image runs it: boots the live ISO in a VM (3 GB, no
# window), serves this repository's kalangod and imagem/lista-branca-roteiro.py to it (http on 127.0.0.1, which the VM
# reaches as 10.0.2.2), and runs the checks as the live user. The VM is stopped at the end.
#
#   cargo build --release -p kalangod && imagem/testar-lista-branca.sh
#   KALANGOD=caminho/kalangod (padrão: target/release/kalangod) · KALANGO_ISO=outra ISO
set -u
cd "$(dirname "$0")/.."
KD=${KALANGOD:-$PWD/target/release/kalangod}
[ -x "$KD" ] || { echo "falta $KD: cargo build --release -p kalangod"; exit 1; }
# (a short folder: the serial and monitor sockets' paths must fit in a Unix socket address)
D=$(mktemp -d /tmp/kalango-vm-XXXX)
echo "teste da lista branca em $D (o console: $D/console.log)"
mkdir -p "$D/servir" && cp "$KD" "$D/servir/kalangod" && cp imagem/lista-branca-roteiro.py "$D/servir/roteiro.py" && cp crates/kalangod/tests/fixtures/conector_teste.py "$D/servir/"
python3 -I -m http.server 18432 --bind 127.0.0.1 --directory "$D/servir" > "$D/http.log" 2>&1 &
HTTP=$!
VM=
parar() {
  [ -S "$D/monitor.sock" ] && python3 -I -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.settimeout(5); s.recv(4096); s.send(b"quit\n")' "$D/monitor.sock" 2>/dev/null
  [ -n "$VM" ] && kill "$VM" 2>/dev/null
  kill "$HTTP" 2>/dev/null
  wait 2>/dev/null || true
}
trap parar EXIT
VM_DIR=$D VM_TELA=none MEM=${MEM:-3072} CPUS=${CPUS:-2} DISCO=4G imagem/rodar-iso.sh iso > "$D/qemu.log" 2>&1 &
VM=$!
for _ in $(seq 60); do [ -S "$D/serial.sock" ] && break; sleep 1; done
[ -S "$D/serial.sock" ] || { echo "a VM não ligou:"; cat "$D/qemu.log"; exit 1; }
python3 -I - "$D/serial.sock" "$PWD/imagem/testar-iso.py" <<'PY'
import importlib.util, sys
spec = importlib.util.spec_from_file_location("t", sys.argv[2]); t = importlib.util.module_from_spec(spec); spec.loader.exec_module(t)
c = t.Console(sys.argv[1])
c.login("kalango", "", 900)
c.run("python3 -c \"import urllib.request as u; [u.urlretrieve('http://10.0.2.2:18432/'+n, '/tmp/'+n) for n in ('kalangod', 'roteiro.py', 'conector_teste.py')]\" && chmod +x /tmp/kalangod", 120)
out, code = c.run("python3 /tmp/roteiro.py 2>&1", 900)
print(out)
sys.exit(code)
PY
R=$?
exit $R
