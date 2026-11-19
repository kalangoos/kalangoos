#!/bin/bash
# KalangoOS · testa o kalangod como root numa VM descartável, com o eBPF LSM ligado.
#
# A VM usa o kernel do host (ou o indicado em KERNEL=) e enxerga a raiz do host só para leitura (9p); tudo que o
# teste muda vai para uma camada overlay em memória e some quando a VM desliga. Nada no host é alterado.
#
#   sudo vm/run.sh              # roda os testes de vm/dentro.sh
#   sudo KVER=7.3.4-kalango KERNEL=…/vmlinuz MODBASE=…/raiz-do-pacote LSM= vm/run.sh
#        (outro kernel: MODBASE é a pasta onde o .deb foi extraído; LSM vazio não passa lsm= e usa o padrão do kernel)
set -euo pipefail
cd "$(dirname "$0")/.."
[ "$(id -u)" = 0 ] || { echo "rode com sudo (a VM precisa ler a raiz do host como root)"; exit 1; }
KVER=${KVER:-$(uname -r)}
KERNEL=${KERNEL:-/boot/vmlinuz-$KVER}
MODBASE=${MODBASE:-/}
MODS="$MODBASE/lib/modules/$KVER"
LSM=${LSM-landlock,lockdown,yama,integrity,apparmor,bpf}
OUT=target/vm
rm -rf "$OUT/initrd" && mkdir -p "$OUT/initrd"/{bin,mods,proc,sys,dev,host,mnt}
cp /usr/bin/busybox "$OUT/initrd/bin/"
for m in netfs 9pnet 9pnet_virtio 9p overlay; do
  f=$(modinfo -b "$MODBASE" -k "$KVER" -n "$m" 2>/dev/null || true)
  case "$f" in
    *.zst) zstd -dqf "$f" -o "$OUT/initrd/mods/$m.ko" ;;
    *.ko) cp "$f" "$OUT/initrd/mods/$m.ko" ;;
    *) : ;; # embutido no kernel
  esac
done
cp vm/init "$OUT/initrd/init" && chmod +x "$OUT/initrd/init"
(cd "$OUT/initrd" && find . | cpio -o -H newc --quiet | gzip -1) > "$OUT/initrd.gz"
TESTE=$(realpath vm/dentro.sh)
LOG="$OUT/vm.log"
timeout 900 qemu-system-x86_64 -enable-kvm -cpu host -m 4096 -smp 4 -nographic -no-reboot \
  -kernel "$KERNEL" -initrd "$OUT/initrd.gz" \
  -append "console=ttyS0 quiet panic=-1 ${LSM:+lsm=$LSM} kalango_teste=$TESTE" \
  -virtfs local,path=/,mount_tag=hostroot,security_model=passthrough,readonly=on,multidevs=remap \
  -virtfs local,path="$MODS",mount_tag=modulos,security_model=passthrough,readonly=on \
  -nic user,model=virtio-net-pci 2>&1 | tee "$LOG"
grep -q "KALANGO-VM: TUDO OK" "$LOG"
