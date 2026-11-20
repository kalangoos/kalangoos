#!/bin/bash
# KalangoOS · liga a ISO (target/iso/kalangoos.iso) numa VM com firmware UEFI e um disco vazio, como um computador
# novo: para experimentar a sessão ao vivo e instalar. Sem root e sem tocar nos discos do computador hospedeiro.
#
#   imagem/rodar-iso.sh                  a ISO e o disco virtual: o disco vem primeiro no boot (instalado, liga o
#                                        sistema; vazio, cai na ISO, como num computador novo com o pendrive)
#   imagem/rodar-iso.sh novo             apaga o disco virtual antes (uma instalação do zero)
#   imagem/rodar-iso.sh instalado        só o disco virtual, sem a ISO
#   RES=1920x1080 MEM=8192 CPUS=6 DISCO=40G (opcionais) · VM_TELA=none: sem janela
#   VM_DIR=pasta    o disco, o firmware e os sockets desta VM (padrão: target/iso; o teste usa a sua, para rodar ao
#                   lado de uma VM aberta)
#   socat - UNIX-CONNECT:target/iso/serial.sock    o console serial (a conta ao vivo "kalango" não tem senha)
set -euo pipefail
cd "$(dirname "$0")/.."
ISO=$PWD/target/iso/kalangoos.iso
[ -f "$ISO" ] || { echo "falta $ISO: KALANGO_IMAGEM_ISO=1 imagem/construir.sh"; exit 1; }
OUT=${VM_DIR:-$PWD/target/iso}
mkdir -p "$OUT"
OVMF_CODE=/usr/share/OVMF/OVMF_CODE_4M.fd
OVMF_VARS=/usr/share/OVMF/OVMF_VARS_4M.fd
[ -f "$OVMF_CODE" ] || { echo "falta o firmware UEFI da VM ($OVMF_CODE: pacote ovmf)"; exit 1; }
MODO=${1:-iso}
# (o disco só é apagado quando se pede: a VM fechada não leva a instalação junto)
if [ "$MODO" = novo ]; then
  rm -f "$OUT/disco.qcow2" "$OUT/uefi-vars.fd"
  MODO=iso
fi
[ -f "$OUT/disco.qcow2" ] || qemu-img create -q -f qcow2 "$OUT/disco.qcow2" "${DISCO:-40G}"
[ -f "$OUT/uefi-vars.fd" ] || cp "$OVMF_VARS" "$OUT/uefi-vars.fd"
RES=${RES:-1920x1080}
TELA=${VM_TELA:-sdl}
if [ "$TELA" = none ]; then
  DISP=(-display none -device "virtio-vga,xres=${RES%x*},yres=${RES#*x}")
else
  DISP=(-device "virtio-vga-gl,xres=${RES%x*},yres=${RES#*x},edid=on" -display "$TELA,gl=on")
fi
CD=()
[ "$MODO" = iso ] && CD=(-drive "file=$ISO,media=cdrom,readonly=on,if=none,id=cd" -device "ide-cd,drive=cd,bootindex=1")
echo "VM ($MODO): ISO $(du -h --apparent-size "$ISO" | cut -f1), disco ${DISCO:-40G}, ${RES}; console em $OUT/console.log"
KVM=(-enable-kvm -cpu host)
[ -w /dev/kvm ] || { KVM=(-cpu max); echo "sem /dev/kvm: a VM roda emulada (lenta)"; }
exec qemu-system-x86_64 -name "KalangoOS (ISO)" "${KVM[@]}" -machine q35 -m "${MEM:-8192}" -smp "${CPUS:-6}" \
  -drive "if=pflash,format=raw,readonly=on,file=$OVMF_CODE" -drive "if=pflash,format=raw,file=$OUT/uefi-vars.fd" \
  "${CD[@]}" \
  -drive "file=$OUT/disco.qcow2,format=qcow2,if=none,id=disco" -device "virtio-blk-pci,drive=disco,bootindex=0" \
  "${DISP[@]}" \
  -device virtio-keyboard-pci -device virtio-tablet-pci \
  -nic user,model=virtio-net-pci \
  -chardev socket,id=serial0,path="$OUT/serial.sock",server=on,wait=off,logfile="$OUT/console.log" -serial chardev:serial0 \
  -monitor unix:"$OUT/monitor.sock",server,nowait
