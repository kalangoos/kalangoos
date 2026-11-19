#!/bin/bash
# KalangoOS · a sessão inteira numa VM descartável, numa janela do seu desktop: o kernel do KalangoOS, o systemd do
# host (raiz só leitura, por 9p), a camada com o KalangoOS "instalado" (preparar-camada.sh) e uma camada em memória
# para o que mudar. A VM entra sozinha na sessão KalangoOS (niri na GPU virtual, shell, dock, kalangod como serviço do
# sistema). Nada no host é alterado; ao desligar, tudo some.
#
#   sudo sessao/vm/rodar-vm.sh            (kernel 7.3.4-kalango de kernel/out; KERNEL_DEB= para outro pacote)
#   RES=1920x1080 MEM=8192 CPUS=6         (opcionais)
# A GPU virtual é a com 3D (virgl, pela placa do host): o niri não desenha com renderizador de software (llvmpipe).
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT=$PWD
[ "$(id -u)" = 0 ] || { echo "rode com sudo (a VM precisa ler a raiz do host como root)"; exit 1; }
PESSOA=${SUDO_USER:-$(logname)}
OUT=$ROOT/target/vm-grafica
mkdir -p "$OUT" && chown "$PESSOA": "$OUT"
# o kernel do KalangoOS (o pacote extraído; nada é instalado no host)
DEB=${KERNEL_DEB:-$(ls kernel/out/linux-image-*-kalango_*_amd64.deb | grep -v dbg | head -1)}
K="$OUT/kernel"
rm -rf "$K" && mkdir -p "$K" && dpkg -x "$DEB" "$K"
KVER=$(ls "$K/lib/modules")
# a camada do sistema instalado, feita pela pessoa (os arquivos ficam dela)
RES=${RES:-1920x1080}
mkdir -p "$OUT/saida"
sudo -u "$PESSOA" env KALANGO_VM_PESSOA="$PESSOA" KALANGO_VM_RES="$RES" sessao/vm/preparar-camada.sh "$OUT/camada"
# o initrd: busybox e os módulos para montar a raiz
rm -rf "$OUT/initrd" && mkdir -p "$OUT/initrd"/{bin,mods,proc,sys,dev,host,kalango,mnt}
cp /usr/bin/busybox "$OUT/initrd/bin/"
for m in netfs 9pnet 9pnet_virtio 9p overlay; do
  f=$(modinfo -b "$K" -k "$KVER" -n "$m" 2>/dev/null || true)
  case "$f" in
    *.zst) zstd -dqf "$f" -o "$OUT/initrd/mods/$m.ko" ;;
    *.ko) cp "$f" "$OUT/initrd/mods/$m.ko" ;;
    *) : ;;
  esac
done
cp sessao/vm/init "$OUT/initrd/init" && chmod +x "$OUT/initrd/init"
(cd "$OUT/initrd" && find . | cpio -o -H newc --quiet | gzip -1) > "$OUT/initrd.gz"
RES=${RES:-1920x1080}
echo "VM: kernel $KVER, ${RES}, registro do console em $OUT/console.log"
# a janela no desktop de quem chamou
export DISPLAY=${DISPLAY:-:0}
export XAUTHORITY=${XAUTHORITY:-/run/user/$(id -u "$PESSOA")/gdm/Xauthority}
exec qemu-system-x86_64 -name "KalangoOS (VM · kernel $KVER)" -enable-kvm -cpu host -machine q35 -m "${MEM:-8192}" -smp "${CPUS:-6}" \
  -kernel "$K/boot/vmlinuz-$KVER" -initrd "$OUT/initrd.gz" \
  -append "console=ttyS0 loglevel=3 systemd.show_status=false" \
  -virtfs local,path=/,mount_tag=hostroot,security_model=passthrough,readonly=on,multidevs=remap \
  -virtfs local,path="$K/lib/modules/$KVER",mount_tag=modulos,security_model=passthrough,readonly=on \
  -virtfs local,path="$OUT/camada",mount_tag=kalango,security_model=passthrough,readonly=on,multidevs=remap \
  -device virtio-vga-gl,xres=${RES%x*},yres=${RES#*x},edid=on -display ${VM_TELA:-sdl},gl=on \
  -monitor unix:"$OUT/monitor.sock",server,nowait \
  -virtfs local,path="$OUT/saida",mount_tag=saida,security_model=none \
  -device virtio-keyboard-pci -device virtio-tablet-pci \
  -nic user,model=virtio-net-pci -serial file:"$OUT/console.log"
