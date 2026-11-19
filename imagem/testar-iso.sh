#!/bin/bash
# KalangoOS · testa a ISO numa VM sem tela (a CI roda este; aqui roda ao lado de uma VM aberta):
#
#   imagem/testar-iso.sh            a sessão ao vivo sobe, com o instalador, e fica de pé; nenhum serviço falha
#   imagem/testar-iso.sh instalar   e mais: instala num disco virtual vazio (o motor de verdade, apagando o disco da
#                                   VM), liga pelo disco instalado e confere a sessão da conta criada
#
# Cada execução usa uma pasta nova em target/iso/teste-* (o disco, o firmware, o console); ela fica para ler depois.
set -euo pipefail
cd "$(dirname "$0")/.."
D=$(mktemp -d "$PWD/target/iso/teste-XXXX")
echo "teste da ISO em $D (o console: $D/console.log)"
VM=
parar() { [ -n "$VM" ] && kill "$VM" 2>/dev/null; wait 2>/dev/null || true; }
trap parar EXIT
ligar() {  # ligar iso|instalado
  rm -f "$D/serial.sock"
  VM_DIR=$D VM_TELA=none DISCO_NOVO=0 MEM=${MEM:-6144} CPUS=${CPUS:-4} imagem/rodar-iso.sh "$1" > "$D/qemu-$1.log" 2>&1 &
  VM=$!
  for _ in $(seq 60); do [ -S "$D/serial.sock" ] && return; sleep 1; done
  echo "a VM não ligou:"; cat "$D/qemu-$1.log"; exit 1
}
esperar_desligar() {
  for _ in $(seq 45); do kill -0 "$VM" 2>/dev/null || { VM=; return; }; sleep 2; done
  # (ao vivo, o casper espera "tire a mídia e aperte ENTER" ao desligar; o disco já foi gravado e desmontado: a VM sai
  # pelo monitor do QEMU, que grava o que tiver em cache no disco)
  python3 -I -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.send(b"quit\n")' "$D/monitor.sock" 2>/dev/null || true
  for _ in $(seq 30); do kill -0 "$VM" 2>/dev/null || { VM=; return; }; sleep 2; done
  echo "a VM não desligou"; exit 1
}

ligar iso
python3 -I imagem/testar-iso.py "$D/serial.sock" ao-vivo
[ "${1:-}" = instalar ] || exit 0

# a conta da instalação de teste (só nesta VM)
SENHA="teste-$(date +%s)"
cat > "$D/plano.json" <<PLANO
{"disk": "/dev/vda", "disk_title": "Disco virtual", "mode": "Erase", "size": 0, "encrypt": false,
 "locale": "pt_BR.UTF-8", "keyboard": "br", "name": "Ana Teste", "user": "ana", "hostname": "teste-kalango",
 "autologin": true, "fingerprint_later": false, "avatar": null}
PLANO
python3 -I imagem/testar-iso.py "$D/serial.sock" instalar "$D/plano.json" "$SENHA"
esperar_desligar
ligar instalado
python3 -I imagem/testar-iso.py "$D/serial.sock" instalado ana "$SENHA"
esperar_desligar
echo "a ISO instala e o sistema instalado liga"
