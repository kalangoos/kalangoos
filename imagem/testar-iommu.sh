#!/bin/bash
# KalangoOS · o IOMMU pelo kalangod numa VM com o KalangoOS instalado: instala a ISO num disco virtual vazio (como o
# testar-iso.sh instalar), liga pelo disco, roda gpu.iommu_ligar de um kalangod deste repositório (servido à VM pela
# rede da própria VM), reinicia e confere /proc/cmdline; depois gpu.iommu_desligar, reinicia e confere de novo.
#
#   cargo build --release -p kalangod && imagem/testar-iommu.sh
#   KALANGOD=caminho/kalangod (padrão: target/release/kalangod)
set -euo pipefail
cd "$(dirname "$0")/.."
KD=${KALANGOD:-$PWD/target/release/kalangod}
[ -x "$KD" ] || { echo "falta $KD: cargo build --release -p kalangod"; exit 1; }
D=$(mktemp -d "$PWD/target/iso/teste-iommu-XXXX")
echo "teste do IOMMU em $D (o console: $D/console.log)"
mkdir -p "$D/servir" && cp "$KD" "$D/servir/kalangod" && cp imagem/chamar-kalangod.py "$D/servir/"
# (a VM alcança este computador em 10.0.2.2 pela rede do QEMU; só esta pasta é servida, e só para 127.0.0.1)
python3 -I -m http.server 18431 --bind 127.0.0.1 --directory "$D/servir" > "$D/http.log" 2>&1 &
HTTP=$!
VM=
parar() {
  [ -S "$D/monitor.sock" ] && python3 -I -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.settimeout(5); s.recv(4096); s.send(b"quit\n")' "$D/monitor.sock" 2>/dev/null
  [ -n "$VM" ] && kill "$VM" 2>/dev/null
  kill "$HTTP" 2>/dev/null
  wait 2>/dev/null || true
}
trap parar EXIT
ligar() {
  rm -f "$D/serial.sock"
  VM_DIR=$D VM_TELA=none MEM=${MEM:-6144} CPUS=${CPUS:-4} imagem/rodar-iso.sh "$1" > "$D/qemu-$1.log" 2>&1 &
  VM=$!
  for _ in $(seq 60); do [ -S "$D/serial.sock" ] && return; sleep 1; done
  echo "a VM não ligou:"; cat "$D/qemu-$1.log"; exit 1
}
esperar_desligar() {
  for _ in $(seq 60); do kill -0 "$VM" 2>/dev/null || { VM=; return; }; sleep 2; done
  python3 -I -c '
import socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.settimeout(10)
buf = b""
while b"(qemu)" not in buf:
    buf += s.recv(4096)
s.send(b"quit\n"); s.recv(4096)' "$D/monitor.sock" 2>/dev/null || true
  for _ in $(seq 30); do kill -0 "$VM" 2>/dev/null || { VM=; return; }; sleep 2; done
  echo "a VM não desligou"; exit 1
}
SENHA="teste-$(date +%s)"
cat > "$D/plano.json" <<PLANO
{"disk": "/dev/vda", "disk_title": "Disco virtual", "mode": "Erase", "size": 0, "encrypt": false,
 "locale": "pt_BR.UTF-8", "keyboard": "br", "name": "Ana Teste", "user": "ana", "hostname": "teste-kalango",
 "autologin": true, "fingerprint_later": false, "avatar": null}
PLANO
ligar iso
python3 -I imagem/testar-iso.py "$D/serial.sock" instalar "$D/plano.json" "$SENHA"
esperar_desligar
ligar instalado
python3 -I imagem/testar-iso.py "$D/serial.sock" iommu ana "$SENHA" http://10.0.2.2:18431
esperar_desligar
echo "o IOMMU liga e desliga pelo kalangod"
