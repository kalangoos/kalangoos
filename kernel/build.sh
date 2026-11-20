#!/bin/sh
# KalangoOS · compila o kernel estável mais recente do kernel.org com a configuração do Ubuntu + kalango.config
# e gera pacotes .deb em kernel/out/. NÃO instala nada.
#
#   ./build.sh            usa a versão estável mais recente (consulta kernel.org)
#   ./build.sh 7.3.4      versão específica
#
# Com kernel/config-<versão> no repositório (a configuração final de uma compilação anterior), ela é a base: o mesmo
# kernel em qualquer máquina, inclusive na CI. Sem ela, a base é a do Ubuntu que está rodando (/boot/config-*).
# KALANGO_KERNEL_JOBS limita os núcleos; KALANGO_KERNEL_SEM_DBG=1 não guarda o pacote de depuração (1,6 GB).
#
# Requisitos: flex bison bc pahole(dwarves) libelf-dev libssl-dev libdw-dev debhelper rsync zstd
set -eu
cd "$(dirname "$0")"
# (caminhos absolutos: kernel/src pode ser um link para outro disco, como na CI, e lá "../.." sairia do repositório)
K=$(pwd)
V="${1:-$(curl -s https://www.kernel.org/releases.json | python3 -c 'import json,sys;print(json.load(sys.stdin)["latest_stable"]["version"])')}"
MAJOR="${V%%.*}"
URL="https://cdn.kernel.org/pub/linux/kernel/v$MAJOR.x"
# ferramentas de compilação (o passo final, modules.builtin.ranges, precisa do gawk)
for t in gawk flex bison bc pahole dpkg-buildpackage; do
  command -v "$t" >/dev/null || { echo "falta $t (sudo apt install gawk flex bison bc dwarves dpkg-dev)"; exit 1; }
done
mkdir -p src out
cd src
if [ ! -d "linux-$V" ]; then
  echo ">> baixando linux-$V"
  curl -fsSLO "$URL/linux-$V.tar.xz"
  curl -fsSLO "$URL/sha256sums.asc"
  grep " linux-$V.tar.xz\$" sha256sums.asc | sha256sum -c -
  tar -xf "linux-$V.tar.xz"
fi
cd "linux-$V"
if [ -f "$K/config-$V" ]; then
  echo ">> configuração: kernel/config-$V + kalango.config"
  cp "$K/config-$V" .config
else
  echo ">> configuração: Ubuntu ($(uname -r)) + kalango.config"
  cp "/boot/config-$(uname -r)" .config
fi
# chaves de assinatura do Ubuntu não estão disponíveis fora da Canonical: o kernel gera as próprias
scripts/config --set-str SYSTEM_TRUSTED_KEYS "" --set-str SYSTEM_REVOCATION_KEYS "" --disable DEBUG_INFO_DWARF5 --enable DEBUG_INFO_DWARF4
./scripts/kconfig/merge_config.sh -m .config "$K/kalango.config" >/dev/null
make olddefconfig >/dev/null
for opt in BPF_LSM SECURITY_LANDLOCK DEBUG_INFO_BTF FANOTIFY_ACCESS_PERMISSIONS CGROUP_BPF; do
  grep -q "^CONFIG_$opt=y" .config || { echo "faltou CONFIG_$opt"; exit 1; }
done
grep '^CONFIG_LSM=' .config
J=${KALANGO_KERNEL_JOBS:-$(nproc)}
echo ">> compilando com $J núcleos (leva de 30 a 90 min)"
nice -n 10 make -j"$J" bindeb-pkg LOCALVERSION= KDEB_PKGVERSION="$V-1kalango" >"$K/out/build-$V.log" 2>&1
[ "${KALANGO_KERNEL_SEM_DBG:-0}" = 1 ] && rm -f ../linux-image-*-dbg_*.deb
mv ../linux-*"$V"*.deb "$K/out/" 2>/dev/null || true
ls -la "$K"/out/*.deb
echo ">> pronto. Para instalar (exige reinício): sudo apt install ./out/linux-image-$V*.deb ./out/linux-headers-$V*.deb"
