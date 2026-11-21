#!/bin/bash
# KalangoOS · constrói a imagem do sistema: uma distribuição própria e mínima.
#
# A raiz vem dos pacotes binários do Ubuntu 26.04 LTS — só os de que a pilha do KalangoOS precisa, sem documentação,
# manuais nem traduções além de pt_BR e inglês —, com Rust onde já é maduro (coreutils do uutils, sudo-rs, ntpd-rs).
# Por cima: o kernel do KalangoOS, o niri com os patches, o shell, o kalangod, o agente, a aprovação, a tela de entrada
# e o instalador, como arquivos da imagem (nada aponta para o repositório).
#
# Sem root: `mmdebstrap --mode=unshare` (o Ubuntu traz o perfil do AppArmor que libera user namespaces para ele) e
# `fakeroot` para o disco ext4. Saídas em target/imagem/:
#   kalangoos.tar        a raiz
#   kalangoos.squashfs   o que o instalador grava (passo 2 do plano)
#   kalangoos.img        um disco ext4 (cru) que a VM liga: imagem/rodar-vm.sh
#   vmlinuz, initrd.img  o kernel e o initramfs da imagem (a VM liga por eles, sem carregador de boot)
#   tamanhos.txt         o tamanho de cada parte (a imagem é medida a cada construção)
#
#   imagem/construir.sh                    (a sessão entra sozinha: imagem de teste)
#   KALANGO_IMAGEM_LOGIN=1 imagem/construir.sh   (abre a tela de entrada)
#   KALANGO_IMAGEM_ISO=1 imagem/construir.sh     a ISO para instalar (target/iso/kalangoos.iso): sem conta nenhuma;
#                                         liga ao vivo (casper) e abre o instalador; BIOS e UEFI
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD
ISO=${KALANGO_IMAGEM_ISO:-0}
if [ "$ISO" = 1 ]; then KALANGO_IMAGEM_TESTE=0; OUT=$ROOT/target/iso; else OUT=$ROOT/target/imagem; fi
mkdir -p "$OUT"
# (o root dentro do namespace é outro usuário: a pasta de trabalho precisa ser atravessável por ele)
W=${KALANGO_IMAGEM_TMP:-/tmp/kalango-imagem-build}
mkdir -p "$W" && chmod 755 "$W"
SUITE=${KALANGO_IMAGEM_SUITE:-resolute}
MIRROR=${KALANGO_IMAGEM_MIRROR:-http://archive.ubuntu.com/ubuntu}
DEPS=$ROOT/poc/.deps
for f in target/release/kalango-shell target/release/kalangod target/release/kalango-agente target/release/kalango-aprovacao \
         target/release/kalango-entrada target/release/kalango-instalador "$DEPS/src/niri/target/release/niri"; do
  [ -x "$f" ] || { echo "falta $f: compile antes (cargo build --release; o niri em poc/.deps/src/niri)"; exit 1; }
done
KDEB=${KERNEL_DEB:-$(ls kernel/out/linux-image-*-kalango_*_amd64.deb 2>/dev/null | grep -v dbg | head -1)}
[ -f "$KDEB" ] || { echo "falta o pacote do kernel em kernel/out (kernel/build.sh)"; exit 1; }

# ---- a camada do KalangoOS: os arquivos nos lugares da imagem
C=$W/camada
rm -rf "$C" && mkdir -p "$C"
d() { mkdir -p "$C/$1"; }
d usr/bin; d usr/libexec/kalango; d usr/lib/systemd/user/graphical-session.target.wants; d usr/share/kalango/papeis-de-parede
d usr/share/wayland-sessions; d usr/lib/environment.d; d etc/kalango; d etc/systemd/system/multi-user.target.wants; d etc/greetd
# (o niri foi compilado no Ubuntu 24.04 e usa a libdisplay-info.so.1; o 26.04 tem a .so.3: vai junto, só para ele,
# até os nossos binários serem compilados na própria base)
d usr/lib/kalango
DI=$(ls "$DEPS"/root/usr/lib/x86_64-linux-gnu/libdisplay-info.so.1 /usr/lib/x86_64-linux-gnu/libdisplay-info.so.1 2>/dev/null | head -1 || true)
[ -n "$DI" ] || { echo "falta a libdisplay-info.so.1 (a do Ubuntu 24.04, com que o niri foi compilado)"; exit 1; }
cp -L "$DI" "$C/usr/lib/kalango/"
install -m 755 "$DEPS/src/niri/target/release/niri" "$C/usr/libexec/kalango/niri.bin"
printf '#!/bin/sh\nexec env LD_LIBRARY_PATH=/usr/lib/kalango /usr/libexec/kalango/niri.bin "$@"\n' > "$C/usr/bin/niri"
chmod 755 "$C/usr/bin/niri"
install -m 755 "$DEPS/src/niri/resources/niri-session" "$C/usr/bin/niri-session"
cp "$DEPS/src/niri/resources/niri.service" "$DEPS/src/niri/resources/niri-shutdown.target" "$C/usr/lib/systemd/user/"
sed -i 's#^ExecStart=niri --session#ExecStart=/usr/bin/niri --session#' "$C/usr/lib/systemd/user/niri.service"
for b in kalango-shell kalango-aprovacao kalangod kalango-agente kalango-entrada kalango-instalador; do
  install -m 755 "target/release/$b" "$C/usr/libexec/kalango/$b"
done
# (o shell usa a camada do gtk4-layer-shell: o pacote libgtk4-layer-shell0 da imagem, pré-carregado)
for b in kalango-shell kalango-aprovacao; do
  printf '#!/bin/sh\nexec env LD_PRELOAD=libgtk4-layer-shell.so.0 /usr/libexec/kalango/%s.bin "$@"\n' "$b" > "$C/usr/bin/$b"
  mv "$C/usr/libexec/kalango/$b" "$C/usr/libexec/kalango/$b.bin"
  ln -s ../../bin/$b "$C/usr/libexec/kalango/$b"
  chmod 755 "$C/usr/bin/$b"
done
ln -s ../libexec/kalango/kalango-agente "$C/usr/bin/kalango-agente"
ln -s ../libexec/kalango/kalango-instalador "$C/usr/bin/kalango-instalador"
cp sessao/dist/kalango-sessao sessao/dist/kalango-entrada-sessao "$C/usr/libexec/kalango/"
cp sessao/dist/entrada.kdl "$C/usr/share/kalango/"
sessao/gerar.sh > "$C/usr/share/kalango/niri.kdl"
printf '\ninclude "~/.config/kalango/atalhos.kdl" optional=true\n' >> "$C/usr/share/kalango/niri.kdl"
cp sessao/dist/kalango.desktop "$C/usr/share/wayland-sessions/"
cp sessao/dist/kalango-shell.service crates/kalango-aprovacao/dist/kalango-aprovacao.service "$C/usr/lib/systemd/user/"
sed -i 's#ExecStart=[^ ]*kalango-shell#ExecStart=/usr/bin/kalango-shell#; s#ExecStart=[^ ]*kalango-aprovacao#ExecStart=/usr/bin/kalango-aprovacao#' \
  "$C/usr/lib/systemd/user/kalango-shell.service" "$C/usr/lib/systemd/user/kalango-aprovacao.service"
ln -s ../kalango-shell.service "$C/usr/lib/systemd/user/graphical-session.target.wants/kalango-shell.service"
ln -s ../kalango-aprovacao.service "$C/usr/lib/systemd/user/graphical-session.target.wants/kalango-aprovacao.service"
cat > "$C/usr/lib/environment.d/90-kalango.conf" <<'ENV'
KALANGO_MOD_TAP=/usr/libexec/kalango/kalango-shell super tocar
KALANGO_MOD_HOLD=/usr/libexec/kalango/kalango-shell super segurar
KALANGO_MOD_HOLD_END=/usr/libexec/kalango/kalango-shell super soltar
KALANGO_MOD_HOLD_CANCEL=/usr/libexec/kalango/kalango-shell super cancelar
KALANGO_SHELL_CMD=/usr/libexec/kalango/kalango-shell
ENV
cp crates/kalangod/dist/kalangod.toml "$C/etc/kalango/"
cp crates/kalangod/dist/kalangod.service crates/kalangod/dist/kalango-agentes.slice "$C/etc/systemd/system/"
sed -i 's#ExecStart=[^ ]*kalangod#ExecStart=/usr/libexec/kalango/kalangod#' "$C/etc/systemd/system/kalangod.service"
ln -s ../kalangod.service "$C/etc/systemd/system/multi-user.target.wants/kalangod.service"
# os ícones dos apps do KalangoOS (o shell os procura em /usr/share/kalango/icones)
d usr/share/kalango/icones; cp assets/icons/kalango/svg/*.svg "$C/usr/share/kalango/icones/"
# os sons do sistema: o tema freedesktop do KalangoOS (o shell toca os arquivos dele; apps de terceiros pelo libcanberra,
# que escolhe o tema pelo gtk-sound-theme-name ou pelo org.gnome.desktop.sound theme-name: os dois dizem "kalango")
d usr/share/sounds/kalango; cp -r assets/sounds/kalango/index.theme assets/sounds/kalango/stereo assets/sounds/kalango/LEIAME.md "$C/usr/share/sounds/kalango/"
# (o GTK 3: em /etc/xdg, que o GTK lê depois de /etc/gtk-3.0 — esse é do pacote libgtk-3-0t64, que diz "Yaru")
d etc/gtk-4.0; d etc/xdg/gtk-3.0
printf '[Settings]\ngtk-sound-theme-name=kalango\n' | tee "$C/etc/gtk-4.0/settings.ini" > "$C/etc/xdg/gtk-3.0/settings.ini"
d usr/share/glib-2.0/schemas
printf "[org.gnome.desktop.sound]\ntheme-name='kalango'\n" > "$C/usr/share/glib-2.0/schemas/90_kalango-sons.gschema.override"
# o navegador dos agentes (Moli, preso pelo kalangod) e o motor dos modelos locais (llama.cpp: o backend de CPU; o de
# CUDA vem com os drivers da NVIDIA)
[ -x "$DEPS/moli/moli" ] && install -m 755 "$(readlink -f "$DEPS/moli/moli")" "$C/usr/lib/kalango/moli"
# (a compilação da imagem: backends carregados em tempo de execução — as variantes de CPU para qualquer x86-64, e a
# de CUDA só se os drivers da NVIDIA estiverem — em vez da do computador de desenvolvimento, ligada à CUDA)
LL=$DEPS/src/llama.cpp/build-imagem/bin
[ -x "$LL/llama-server" ] || { echo "falta o llama.cpp da imagem: cmake -B build-imagem -DGGML_BACKEND_DL=ON -DGGML_CPU_ALL_VARIANTS=ON -DGGML_NATIVE=OFF -DGGML_CUDA=OFF (em $DEPS/src/llama.cpp)"; exit 1; }
d usr/lib/kalango/llama
install -m 755 "$LL/llama-server" "$C/usr/lib/kalango/llama/llama-server"
for f in "$LL"/lib*.so*; do case "$f" in *cuda*) ;; *) cp -a "$f" "$C/usr/lib/kalango/llama/";; esac; done
printf '#!/bin/sh\nexec env LD_LIBRARY_PATH=/usr/lib/kalango/llama /usr/lib/kalango/llama/llama-server "$@"\n' > "$C/usr/lib/kalango/llama-server"
chmod 755 "$C/usr/lib/kalango/llama-server"
# o modelo de texto (LFM3 1.7B): o agente funciona sem internet desde o primeiro uso. Voz e visão: Central de Apps
MODELOS=${KALANGO_MODELOS:-$HOME/.local/share/kalango/modelos}
d usr/share/kalango/modelos
[ -f "$MODELOS/LFM3-1.7B-Q4_K_M.gguf" ] && cp "$MODELOS/LFM3-1.7B-Q4_K_M.gguf" "$C/usr/share/kalango/modelos/"
# ---- os apps nativos que têm repositório próprio (kalangoos/kalango-remote e kalangoos/kalango-audio), compilados ao
# lado deste repositório (KALANGO_REMOTE_DIR / KALANGO_AUDIO_DIR mudam onde); KALANGO_IMAGEM_SEM_APPS=1 monta sem eles
REMOTO=${KALANGO_REMOTE_DIR:-$ROOT/../kalango-remote}
AUDIO=${KALANGO_AUDIO_DIR:-$ROOT/../kalango-audio}
if [ "${KALANGO_IMAGEM_SEM_APPS:-0}" != 1 ]; then
  for f in "$REMOTO/target/release/kalango-remote" "$REMOTO/target/release/kalango-remote-core" "$REMOTO/kalangoos/.deps/bin/cloudflared" \
           "$AUDIO/target/release/kalango-audio"; do
    [ -x "$f" ] || { echo "falta $f: compile o Kalango Remote (kalangoos/scripts/deps.sh, cloudflared.sh e cargo build --release" \
                          "-p kalango-remote -p kalango-remote-core --features kalango-remote-core/linux-pkg-config) e o Kalango Audio" \
                          "(cargo build --release), ou KALANGO_IMAGEM_SEM_APPS=1"; exit 1; }
  done
  # Kalango Remote: o app, o núcleo (o RustDesk, num processo à parte) e o cloudflared juntos — o app acha os outros
  # dois na própria pasta. O núcleo foi compilado no Ubuntu 24.04 e usa a libvpx.so.9; o 26.04 tem a .so.12: vai
  # junto, só para ele (BSD-3), até os binários serem compilados na própria base
  d usr/lib/kalango/remote
  for b in kalango-remote kalango-remote-core; do install -m 755 "$REMOTO/target/release/$b" "$C/usr/lib/kalango/remote/$b"; done
  install -m 755 "$(readlink -f "$REMOTO/kalangoos/.deps/bin/cloudflared")" "$C/usr/lib/kalango/remote/cloudflared"
  # (a do Ubuntu 24.04: nas dependências do próprio Remote — kalangoos/scripts/deps.sh, como na CI —, nas do KalangoOS ou
  # no sistema de quem monta)
  VPX=$(ls "$REMOTO"/kalangoos/.deps/root/usr/lib/x86_64-linux-gnu/libvpx.so.9 "$DEPS"/root/usr/lib/x86_64-linux-gnu/libvpx.so.9 \
           /usr/lib/x86_64-linux-gnu/libvpx.so.9 2>/dev/null | head -1 || true)
  [ -n "$VPX" ] || { echo "falta a libvpx.so.9 (a do Ubuntu 24.04, com que o núcleo do Kalango Remote foi compilado)"; exit 1; }
  cp -L "$VPX" "$C/usr/lib/kalango/remote/"
  printf '#!/bin/sh\nexec env LD_LIBRARY_PATH=/usr/lib/kalango/remote /usr/lib/kalango/remote/kalango-remote "$@"\n' > "$C/usr/bin/kalango-remote"
  chmod 755 "$C/usr/bin/kalango-remote"
  d usr/share/applications
  cat > "$C/usr/share/applications/os.kalango.Remote.desktop" <<'DESK'
[Desktop Entry]
Type=Application
Name=Kalango Remote
GenericName=Acesso remoto
Comment=Compartilhe a tela, controle outro computador e acesse servidores por SSH
Exec=kalango-remote
Icon=os.kalango.Remote
Terminal=false
Categories=Network;RemoteAccess;
Keywords=remoto;compartilhar;tela;ssh;mosh;suporte;
StartupWMClass=os.kalango.Remote
DESK
  # (os tamanhos que o repositório do Remote traz, copiados como estão: nada de PIL na máquina que monta)
  d usr/share/icons/hicolor/128x128/apps
  d usr/share/icons/hicolor/256x256/apps
  install -m 644 "$REMOTO/kalangoos/app/icons/app-128.png" "$C/usr/share/icons/hicolor/128x128/apps/os.kalango.Remote.png"
  install -m 644 "$REMOTO/kalangoos/icone.png" "$C/usr/share/icons/hicolor/256x256/apps/os.kalango.Remote.png"
  # Kalango Audio: o binário, o atalho e os ícones do próprio repositório
  install -m 755 "$AUDIO/target/release/kalango-audio" "$C/usr/bin/kalango-audio"
  sed "s|^Exec=.*|Exec=kalango-audio|" "$AUDIO/dist/os.kalango.Audio.desktop" > "$C/usr/share/applications/os.kalango.Audio.desktop"
  for s in 128 256 512; do
    d usr/share/icons/hicolor/${s}x${s}/apps
    install -m 644 "$AUDIO/assets/icons/hicolor/${s}x${s}/apps/os.kalango.Audio.png" "$C/usr/share/icons/hicolor/${s}x${s}/apps/"
  done
fi
# desligar e reiniciar sem esperar 90 s por um processo que não fecha (uma sessão de terminal com um shell
# interativo segurou um desligamento por 94 s): 15 s para cada serviço e sessão parar, no sistema e nas contas
d etc/systemd/system.conf.d
d etc/systemd/user.conf.d
printf '[Manager]\nDefaultTimeoutStopSec=15s\n' > "$C/etc/systemd/system.conf.d/90-kalangoos.conf"
printf '[Manager]\nDefaultTimeoutStopSec=15s\n' > "$C/etc/systemd/user.conf.d/90-kalangoos.conf"
# os serviços do GRUB que gravam o boot bem-sucedido (grub2-common, grub-initrd-fallback) às vezes falham no começo do
# boot ("não foi possível ler /boot/grub/grubenv: Argumento inválido") e passam rodados depois: tentam de novo
for u in grub2-common grub-initrd-fallback; do
  d "etc/systemd/system/$u.service.d"
  printf '[Unit]\nStartLimitIntervalSec=60\nStartLimitBurst=5\n[Service]\nRestart=on-failure\nRestartSec=5s\n' > "$C/etc/systemd/system/$u.service.d/90-kalangoos.conf"
done
# a rede toda pelo NetworkManager: sem um netplan que diga isso, a regra do Ubuntu (10-globally-managed-devices)
# deixa a placa com fio sem gerência — o cabo ligado, "sem conexão" no instalador e no sistema (visto no VirtualBox)
d etc/netplan
printf 'network:\n  version: 2\n  renderer: NetworkManager\n' > "$C/etc/netplan/01-network-manager-all.yaml"
chmod 600 "$C/etc/netplan/01-network-manager-all.yaml"
# os arquivos do sistema que a sessão espera (portais, aplicativos, catálogos de agentes e da loja, manual)
d usr/share; cp -r sessao/dist/share/. "$C/usr/share/"
d etc/xdg; cp -r sessao/dist/etc/xdg/. "$C/etc/xdg/"
cp wallpapers/sdrustos-16x9-4k.png "$C/usr/share/kalango/papeis-de-parede/padrao.png"
cp wallpapers/sdrustos-16x9-4k.png "$C/usr/share/kalango/papel-de-parede.png"
[ -f wallpapers/sdrustos-16x9-4k-claro.png ] && cp wallpapers/sdrustos-16x9-4k-claro.png "$C/usr/share/kalango/papel-de-parede-claro.png"
# a tela de entrada (greetd); na imagem de teste, a sessão abre sozinha para a conta "kalango"
cp sessao/dist/greetd/config.toml "$C/etc/greetd/config.toml"
if [ "$ISO" != 1 ] && [ "${KALANGO_IMAGEM_LOGIN:-0}" != 1 ]; then
  printf '\n[initial_session]\ncommand = "/usr/libexec/kalango/kalango-sessao"\nuser = "kalango"\n' >> "$C/etc/greetd/config.toml"
fi
# só na imagem de teste (a ISO não leva): fotos da tela e o diagnóstico da sessão para quem roda a VM — uma pasta
# compartilhada com o hospedeiro (imagem/rodar-vm.sh) e o console serial (o QEMU não fotografa a tela com 3D)
if [ "${KALANGO_IMAGEM_TESTE:-1}" = 1 ]; then
  cat > "$C/etc/systemd/system/mnt-saida.mount" <<'UNIT'
[Unit]
Description=KalangoOS (imagem de teste): pasta compartilhada com o hospedeiro
ConditionVirtualization=vm
[Mount]
What=saida
Where=/mnt/saida
Type=9p
Options=trans=virtio,version=9p2000.L,nofail
[Install]
WantedBy=multi-user.target
UNIT
  ln -s ../mnt-saida.mount "$C/etc/systemd/system/multi-user.target.wants/mnt-saida.mount"
  cat > "$C/etc/systemd/system/kalango-teste-fotos.service" <<'UNIT'
[Unit]
Description=KalangoOS (imagem de teste): fotos da tela para o hospedeiro
Requires=mnt-saida.mount
After=mnt-saida.mount
[Service]
ExecStart=/bin/sh -c 'while :; do sleep 15; for s in /run/user/*/wayland-*; do case "$s" in *.lock) ;; *) d=$(dirname "$s"); XDG_RUNTIME_DIR=$d WAYLAND_DISPLAY=$(basename "$s") /usr/bin/grim /mnt/saida/tela.png && break;; esac; done; done'
[Install]
WantedBy=multi-user.target
UNIT
  ln -s ../kalango-teste-fotos.service "$C/etc/systemd/system/multi-user.target.wants/kalango-teste-fotos.service"
  cat > "$C/etc/systemd/system/kalango-teste-diagnostico.service" <<'UNIT'
[Unit]
Description=KalangoOS (imagem de teste): diagnóstico no console serial
After=multi-user.target
[Service]
Type=oneshot
ExecStart=/bin/sh -c 'for t in 30 60; do sleep $t; { echo "=== KALANGO diagnóstico ($t s)"; ls -la /dev/dri; loginctl list-sessions; systemctl --failed --no-pager; journalctl -b --no-pager -u greetd | tail -30; journalctl -b --no-pager _COMM=niri | tail -40; journalctl -b --no-pager -n 40 _SYSTEMD_USER_UNIT=kalango-shell.service; journalctl -b --no-pager -n 20 -u kalangod; } > /dev/ttyS0 2>&1; done'
[Install]
WantedBy=multi-user.target
UNIT
  ln -s ../kalango-teste-diagnostico.service "$C/etc/systemd/system/multi-user.target.wants/kalango-teste-diagnostico.service"
fi
echo "kalangoos" > "$C/etc/hostname"
# a distribuição se apresenta como KalangoOS (a base é o Ubuntu 26.04, dito no ID_LIKE)
d usr/lib
cat > "$C/usr/lib/os-release" <<'OSR'
NAME="KalangoOS"
PRETTY_NAME="KalangoOS (em desenvolvimento)"
ID=kalangoos
ID_LIKE="ubuntu debian"
VERSION="0.1 (em desenvolvimento)"
VERSION_ID="0.1"
VERSION_CODENAME=calango
UBUNTU_CODENAME=resolute
HOME_URL="https://kalangoos.com"
SUPPORT_URL="https://github.com/kalangoos/kalangoos"
BUG_REPORT_URL="https://github.com/kalangoos/kalangoos/issues"
LOGO=kalangoos
OSR
printf 'KalangoOS 0.1 (em desenvolvimento) \\n \\l\n\n' > "$C/etc/issue"
printf 'KalangoOS 0.1\n' > "$C/etc/issue.net"
if [ "$ISO" != 1 ]; then
  printf '/dev/vda\t/\text4\terrors=remount-ro\t0 1\n' > "$C/etc/fstab"
else
  # ---- a sessão ao vivo (só quando o kernel liga com boot=casper: no sistema instalado, nada disto roda). O casper
  # cria a conta ao vivo no boot; este serviço, antes da tela de entrada, abre a sessão dela sozinho, deixa o
  # instalador pedir root pelo pkexec sem senha (a conta ao vivo não tem) e abre o instalador ao entrar. Tudo em /etc,
  # que ao vivo é uma camada na memória: o instalador copia o squashfs, intocado
  : > "$C/etc/fstab"
  cat > "$C/etc/casper.conf" <<'CONF'
export USERNAME="kalango"
export USERFULLNAME="KalangoOS ao vivo"
export HOST="kalangoos"
export BUILD_SYSTEM="Ubuntu"
export FLAVOUR="KalangoOS"
CONF
  cat > "$C/usr/libexec/kalango/kalango-ao-vivo" <<'SH'
#!/bin/sh
# KalangoOS · prepara a sessão ao vivo (kalango-ao-vivo.service, só com boot=casper)
set -e
. /etc/casper.conf
printf '\n[initial_session]\ncommand = "/usr/libexec/kalango/kalango-sessao"\nuser = "%s"\n' "$USERNAME" >> /etc/greetd/config.toml
mkdir -p /etc/polkit-1/rules.d /etc/xdg/autostart
cat > /etc/polkit-1/rules.d/49-kalango-ao-vivo.rules <<RULES
// a sessão ao vivo: o instalador roda o motor como root sem pedir senha (a conta ao vivo não tem uma)
polkit.addRule(function(action, subject) {
    if (action.id == "org.freedesktop.policykit.exec" && subject.user == "$USERNAME" && subject.local && subject.active) {
        return polkit.Result.YES;
    }
});
RULES
cat > /etc/xdg/autostart/os.kalango.Instalador.desktop <<DESK
[Desktop Entry]
Type=Application
Name=Instalar o KalangoOS
Exec=env KALANGO_INSTALADOR_REAL=1 /usr/libexec/kalango/kalango-instalador
NoDisplay=true
DESK
SH
  chmod 755 "$C/usr/libexec/kalango/kalango-ao-vivo"
  cat > "$C/etc/systemd/system/kalango-ao-vivo.service" <<'UNIT'
[Unit]
Description=KalangoOS · sessão ao vivo (conta, instalador)
ConditionKernelCommandLine=boot=casper
Before=greetd.service
After=casper.service
[Service]
Type=oneshot
ExecStart=/usr/libexec/kalango/kalango-ao-vivo
[Install]
WantedBy=multi-user.target
UNIT
  ln -s ../kalango-ao-vivo.service "$C/etc/systemd/system/multi-user.target.wants/kalango-ao-vivo.service"
fi
# o kernel do KalangoOS entra pela própria imagem (dpkg instala e o initramfs-tools gera o initrd)
cp "$KDEB" "$W/kernel.deb"

# a conta da imagem de teste: senha "kalango" (só nela, para entrar pelo console da VM; a ISO não traz conta nenhuma:
# quem instala cria a sua no instalador)
if [ "${KALANGO_IMAGEM_TESTE:-1}" = 1 ]; then SENHA='echo kalango:kalango | chroot "$1" chpasswd'; else SENHA='chroot "$1" passwd -l kalango'; fi
CONTA='chroot "$1" useradd --create-home --shell /bin/bash --groups sudo,video,render,input,audio kalango'
# (a ISO: nenhuma conta na imagem — o casper cria a ao vivo no boot, o instalador cria a de quem instala)
if [ "$ISO" = 1 ]; then CONTA=true; SENHA=true; fi

# ---- os pacotes: o que os nossos programas usam (as bibliotecas que eles carregam) e os serviços que as telas controlam
PAC=$(grep -v '^\s*#' imagem/pacotes.txt | tr -s ' \n' ',' | sed 's/^,//; s/,$//')
# (ao vivo: o casper; o instalador o tira do sistema instalado. O initramfs é refeito depois da camada: leva o
# casper.conf do KalangoOS — a conta ao vivo e o nome do computador)
[ "$ISO" = 1 ] && PAC="$PAC,casper"

# ---- a raiz
rm -f "$OUT/kalangoos.tar"
env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin HOME="$HOME" TMPDIR="$W" /usr/bin/mmdebstrap \
  --mode=unshare --variant=minbase --components=main,universe \
  --dpkgopt='path-exclude=/usr/share/doc/*' --dpkgopt='path-include=/usr/share/doc/*/copyright' \
  --dpkgopt='path-exclude=/usr/share/man/*' --dpkgopt='path-exclude=/usr/share/info/*' \
  --dpkgopt='path-exclude=/usr/share/locale/*' --dpkgopt='path-include=/usr/share/locale/pt_BR/*' \
  --dpkgopt='path-include=/usr/share/locale/pt/*' --dpkgopt='path-include=/usr/share/locale/locale.alias' \
  --include="$PAC" \
  --customize-hook="copy-in '$W/kernel.deb' /tmp" \
  --customize-hook='chroot "$1" dpkg -i /tmp/kernel.deb && rm -f "$1/tmp/kernel.deb"' \
  --customize-hook="sync-in '$C' /" \
  --customize-hook="cd '$C' && find . -print0 | (cd \"\$1\" && xargs -0 chown -h 0:0)" \
  --customize-hook='chroot "$1" useradd --system --create-home --home-dir /var/lib/greeter --shell /usr/sbin/nologin --groups video,render,input greeter || true' \
  --customize-hook='echo "pt_BR.UTF-8 UTF-8" > "$1/etc/locale.gen" && echo "en_US.UTF-8 UTF-8" >> "$1/etc/locale.gen" && chroot "$1" locale-gen && printf "LANG=pt_BR.UTF-8\n" > "$1/etc/default/locale"' \
  --customize-hook='chroot "$1" sh -c "getent group sudo >/dev/null || groupadd --system sudo"' \
  --customize-hook="$CONTA" \
  --customize-hook="$SENHA" \
  --customize-hook='chroot "$1" systemctl enable greetd.service NetworkManager.service bluetooth.service ntpd-rs.service || true' \
  --customize-hook='chroot "$1" systemctl set-default graphical.target' \
  --customize-hook='chroot "$1" systemctl mask getty@tty1.service autovt@tty1.service' \
  --customize-hook='[ ! -x "$1/usr/bin/glib-compile-schemas" ] || chroot "$1" glib-compile-schemas /usr/share/glib-2.0/schemas' \
  --customize-hook='chroot "$1" update-initramfs -u -k all' \
  --customize-hook='ln -sf ../usr/lib/os-release "$1/etc/os-release"; rm -f "$1"/etc/update-motd.d/*; rm -f "$1"/etc/legal; : > "$1/etc/motd"' \
  --customize-hook="rm -f \"\$1/etc/apt/sources.list\" && printf 'Types: deb\\nURIs: http://archive.ubuntu.com/ubuntu\\nSuites: $SUITE $SUITE-updates $SUITE-backports\\nComponents: main restricted universe multiverse\\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\\n\\nTypes: deb\\nURIs: http://security.ubuntu.com/ubuntu\\nSuites: $SUITE-security\\nComponents: main restricted universe multiverse\\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\\n' > \"\$1/etc/apt/sources.list.d/ubuntu.sources\"" \
  --customize-hook='rm -rf "$1"/var/cache/apt/archives/*.deb "$1"/var/lib/apt/lists/*' \
  "$SUITE" "$OUT/kalangoos.tar" "$MIRROR"

# ---- os arquivos do sistema são do root: os nossos entram copiados do computador de quem constrói (uid 1000, que na
# imagem é a conta da pessoa) — uma imagem com /usr da conta deixaria qualquer programa dela trocar o sistema
tar --numeric-owner -tvf "$OUT/kalangoos.tar" | awk '$2 ~ /^1000\// && $6 !~ /^\.\/home\//' > "$W/donos.txt"
if [ -s "$W/donos.txt" ]; then echo "arquivos do sistema com dono errado: $(head -3 "$W/donos.txt" | awk '{print $6}' | tr '\n' ' ')"; exit 1; fi

# ---- toda biblioteca que os nossos programas carregam está na imagem (uma que falte derruba a sessão: o niri sem a
# libseat, na primeira imagem)
tar -tf "$OUT/kalangoos.tar" | grep -oE '[^/]+\.so(\.[0-9]+)*$' | sort -u > "$W/sonames.txt"
# (as bibliotecas do llama.cpp vão na pasta dele: contam como presentes)
ls "$C/usr/lib/kalango/llama" "$C/usr/lib/kalango" "$C/usr/lib/kalango/remote" 2>/dev/null | grep -E '\.so' >> "$W/sonames.txt"; sort -u -o "$W/sonames.txt" "$W/sonames.txt"
for b in "$C"/usr/libexec/kalango/*.bin "$C"/usr/libexec/kalango/kalangod "$C"/usr/libexec/kalango/kalango-agente \
         "$C"/usr/libexec/kalango/kalango-entrada "$C"/usr/libexec/kalango/kalango-instalador \
         "$C"/usr/lib/kalango/llama/llama-server "$C"/usr/lib/kalango/llama/*.so "$C"/usr/lib/kalango/moli \
         "$C"/usr/lib/kalango/remote/kalango-remote "$C"/usr/lib/kalango/remote/kalango-remote-core "$C"/usr/bin/kalango-audio; do
  [ -f "$b" ] || continue
  readelf -d "$b" | sed -n 's/.*NEEDED.*\[\(.*\)\]/\1/p'
  # (e as que eles carregam sozinhos, por nome: o niri abre a libwayland-server assim)
  strings "$b" | grep -E '^lib[a-z0-9_+-]+\.so(\.[0-9]+)*$'
done | sort -u | comm -23 - "$W/sonames.txt" > "$W/faltam.txt"
if [ -s "$W/faltam.txt" ]; then echo "bibliotecas que faltam na imagem: $(tr '\n' ' ' < "$W/faltam.txt") (imagem/pacotes.txt)"; exit 1; fi

# ---- o kernel e o initramfs para a VM ligar direto
KVER=$(tar -tf "$OUT/kalangoos.tar" | sed -n 's#^\./boot/vmlinuz-##p' | head -1)
tar -xf "$OUT/kalangoos.tar" -C "$OUT" --strip-components=2 "./boot/vmlinuz-$KVER" "./boot/initrd.img-$KVER"
mv -f "$OUT/vmlinuz-$KVER" "$OUT/vmlinuz"; mv -f "$OUT/initrd.img-$KVER" "$OUT/initrd.img"

# ---- o squashfs (o instalador grava este) e o disco ext4 da VM (dono e permissões preservados pelo fakeroot)
rm -f "$OUT/kalangoos.squashfs" "$OUT/kalangoos.img"
# (o tar não tem a entrada "./": sem as opções, a raiz sairia de quem monta a imagem, com 777, e o sistema instalado
# teria a / da primeira conta — o systemd-tmpfiles recusa "unsafe path transition" e o resto do sistema confia nela)
sqfstar -quiet -comp zstd -root-uid 0 -root-gid 0 -root-mode 0755 "$OUT/kalangoos.squashfs" < "$OUT/kalangoos.tar"
if [ "$ISO" = 1 ]; then
  # ---- a ISO: o layout do casper e o GRUB para BIOS e UEFI (grub-mkrescue, sem root)
  I=$W/iso; rm -rf "$I"; mkdir -p "$I/casper" "$I/boot/grub" "$I/.disk"
  ln "$OUT/kalangoos.squashfs" "$I/casper/filesystem.squashfs" 2>/dev/null || cp "$OUT/kalangoos.squashfs" "$I/casper/filesystem.squashfs"
  cp "$OUT/vmlinuz" "$I/casper/vmlinuz"; cp "$OUT/initrd.img" "$I/casper/initrd"
  printf '%s\n' "$(du -sb --apparent-size "$OUT/kalangoos.tar" | cut -f1)" > "$I/casper/filesystem.size"
  echo "KalangoOS 0.1 \"calango\" - em desenvolvimento amd64 ($(date +%Y%m%d))" > "$I/.disk/info"
  cat > "$I/boot/grub/grub.cfg" <<'CFG'
set timeout=5
set default=0
insmod all_video
set menu_color_normal=light-gray/black
set menu_color_highlight=black/light-gray
menuentry "Experimentar ou instalar o KalangoOS" {
    linux /casper/vmlinuz boot=casper quiet splash console=ttyS0,115200 console=tty0 ---
    initrd /casper/initrd
}
menuentry "KalangoOS (vídeo seguro: sem o driver da placa)" {
    linux /casper/vmlinuz boot=casper nomodeset console=ttyS0,115200 console=tty0 ---
    initrd /casper/initrd
}
CFG
  # (o casper confere a ISO no boot por esta lista: sem ela, o casper-md5check falha e o resumo do dia aponta)
  (cd "$I" && find . -type f ! -name md5sum.txt -print0 | sort -z | xargs -0 md5sum > md5sum.txt)
  rm -f "$OUT/kalangoos.iso"
  grub-mkrescue -o "$OUT/kalangoos.iso" "$I" -- -volid KALANGOOS 2>&1 | grep -v '^xorriso : UPDATE' || true
  [ -s "$OUT/kalangoos.iso" ] || { echo "a ISO não foi gerada"; exit 1; }
  rm -rf "$I"
  {
    echo "KalangoOS · ISO de $(date +%F) · Ubuntu $SUITE · kernel $KVER"
    echo "pacotes: $(tar -xOf "$OUT/kalangoos.tar" ./var/lib/dpkg/status | grep -c '^Package:')"
    for f in kalangoos.squashfs kalangoos.iso; do printf '%-20s %s\n' "$f" "$(du -h --apparent-size "$OUT/$f" | cut -f1)"; done
  } | tee "$OUT/tamanhos.txt"
  exit 0
fi
TAM=$(( $(stat -c %s "$OUT/kalangoos.tar") / 1048576 * 14 / 10 + 2048 ))
rm -rf "$W/raiz"
fakeroot sh -c "mkdir -p '$W/raiz' && tar -xpf '$OUT/kalangoos.tar' -C '$W/raiz' && mkfs.ext4 -q -L kalangoos -d '$W/raiz' '$OUT/kalangoos.img' ${TAM}M"
rm -rf "$W/raiz"

{
  echo "KalangoOS · imagem de $(date +%F) · Ubuntu $SUITE · kernel $KVER"
  echo "pacotes: $(tar -xOf "$OUT/kalangoos.tar" ./var/lib/dpkg/status | grep -c '^Package:')"
  for f in kalangoos.tar kalangoos.squashfs kalangoos.img vmlinuz initrd.img; do
    printf '%-20s %s\n' "$f" "$(du -h --apparent-size "$OUT/$f" | cut -f1)"
  done
} | tee "$OUT/tamanhos.txt"
