#!/bin/bash
# KalangoOS · liga a imagem construída por imagem/construir.sh numa VM, numa janela do seu desktop. Sem root e sem
# nada do computador hospedeiro: o disco é target/imagem/kalangoos.img (uma cópia descartável dele, a cada vez), o
# kernel e o initramfs são os da própria imagem.
#
#   imagem/rodar-vm.sh                 RES=1920x1080 MEM=8192 CPUS=6 (opcionais)
#   VM_TELA=none imagem/rodar-vm.sh    sem janela (a sessão roda num vídeo virtual; o console serial em
#                                       target/imagem/console.log)
#   socat - UNIX-CONNECT:target/imagem/serial.sock   o console serial, interativo (o login da imagem de teste:
#                                       kalango / kalango)
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=$PWD/target/imagem
for f in kalangoos.img vmlinuz initrd.img; do
  [ -f "$OUT/$f" ] || { echo "falta $OUT/$f: rode imagem/construir.sh"; exit 1; }
done
# (uma cópia: a VM pode mudar o disco à vontade; a imagem construída fica intacta)
cp --sparse=always "$OUT/kalangoos.img" "$OUT/vm-disco.img"
mkdir -p "$OUT/saida"
RES=${RES:-1920x1080}
TELA=${VM_TELA:-sdl}
if [ "$TELA" = none ]; then
  DISP=(-display none -device "virtio-vga,xres=${RES%x*},yres=${RES#*x}")
else
  DISP=(-device "virtio-vga-gl,xres=${RES%x*},yres=${RES#*x},edid=on" -display "$TELA,gl=on")
fi
echo "VM: imagem $(du -h --apparent-size "$OUT/kalangoos.img" | cut -f1), ${RES}; console em $OUT/console.log"
exec qemu-system-x86_64 -name "KalangoOS (imagem)" -enable-kvm -cpu host -machine q35 -m "${MEM:-8192}" -smp "${CPUS:-6}" \
  -kernel "$OUT/vmlinuz" -initrd "$OUT/initrd.img" \
  -append "root=/dev/vda rw console=ttyS0 loglevel=3 systemd.show_status=auto" \
  -drive "file=$OUT/vm-disco.img,format=raw,if=virtio" \
  "${DISP[@]}" \
  -device virtio-keyboard-pci -device virtio-tablet-pci \
  -nic user,model=virtio-net-pci \
  -chardev socket,id=serial0,path="$OUT/serial.sock",server=on,wait=off,logfile="$OUT/console.log" -serial chardev:serial0 \
  -virtfs local,path="$OUT/saida",mount_tag=saida,security_model=none \
  -monitor unix:"$OUT/monitor.sock",server,nowait
