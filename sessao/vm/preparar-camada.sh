#!/bin/bash
# KalangoOS · a camada "sistema instalado" da VM gráfica: os arquivos do KalangoOS nos lugares em que a imagem os põe
# (sessao/README.md), por cima da raiz do host (que a VM vê só para leitura). Roda como a pessoa, sem root.
#   sessao/vm/preparar-camada.sh DESTINO
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT=$PWD
C=${1:?destino}
DEPS=$ROOT/poc/.deps/root
rm -rf "$C" && mkdir -p "$C"
d() { mkdir -p "$C/$1"; }

# ---- o compositor (o niri do KalangoOS, com os patches) e a sessão
d usr/bin; d usr/lib/systemd/user; d usr/libexec/kalango; d usr/lib/kalango; d usr/share/kalango/papeis-de-parede
cp poc/.deps/src/niri/target/release/niri "$C/usr/bin/niri"
cp poc/.deps/src/niri/resources/niri-session "$C/usr/bin/niri-session"
cp poc/.deps/src/niri/resources/niri.service poc/.deps/src/niri/resources/niri-shutdown.target "$C/usr/lib/systemd/user/"
sed -i 's#^ExecStart=niri --session#ExecStart=/usr/bin/niri --session#' "$C/usr/lib/systemd/user/niri.service"
cp sessao/dist/kalango-sessao "$C/usr/libexec/kalango/"
sessao/gerar.sh > "$C/usr/share/kalango/niri.kdl"
# os atalhos que o shell escreve (Configurações › Teclado e atalhos)
printf '\ninclude "~/.config/kalango/atalhos.kdl" optional=true\n' >> "$C/usr/share/kalango/niri.kdl"
# a tela da VM (o monitor virtual do QEMU prefere 640×480)
printf '\noutput "Virtual-1" {\n    mode "%s"\n    scale 1\n}\n' "${KALANGO_VM_RES:-1920x1080}" >> "$C/usr/share/kalango/niri.kdl"
d usr/share/wayland-sessions
cp sessao/dist/kalango.desktop "$C/usr/share/wayland-sessions/"

# ---- o shell, a tela de aprovação e o kalangod (os binários compilados no repositório, vistos pela VM)
cp "$DEPS/usr/lib/x86_64-linux-gnu/"libgtk4-layer-shell.so* "$C/usr/lib/kalango/"
for b in kalango-shell kalango-aprovacao; do
  printf '#!/bin/sh\nexec env LD_PRELOAD=/usr/lib/kalango/libgtk4-layer-shell.so.0 LD_LIBRARY_PATH=/usr/lib/kalango %s/target/release/%s "$@"\n' "$ROOT" "$b" > "$C/usr/libexec/kalango/$b"
  chmod +x "$C/usr/libexec/kalango/$b"
done
ln -s "$ROOT/target/release/kalangod" "$C/usr/libexec/kalango/kalangod"
cp sessao/dist/kalango-shell.service crates/kalango-aprovacao/dist/kalango-aprovacao.service "$C/usr/lib/systemd/user/"
d usr/lib/systemd/user/graphical-session.target.wants
ln -s ../kalango-shell.service "$C/usr/lib/systemd/user/graphical-session.target.wants/kalango-shell.service"
ln -s ../kalango-aprovacao.service "$C/usr/lib/systemd/user/graphical-session.target.wants/kalango-aprovacao.service"
# Super sozinho (patch 0003 do niri): um toque abre a busca, segurar é o modo voz (como a pessoa escolher em
# Configurações › Entrada › Teclado: o shell decide)
d usr/lib/environment.d
cat > "$C/usr/lib/environment.d/90-kalango.conf" <<'ENV'
KALANGO_MOD_TAP=/usr/libexec/kalango/kalango-shell super tocar
KALANGO_MOD_HOLD=/usr/libexec/kalango/kalango-shell super segurar
KALANGO_MOD_HOLD_END=/usr/libexec/kalango/kalango-shell super soltar
KALANGO_MOD_HOLD_CANCEL=/usr/libexec/kalango/kalango-shell super cancelar
KALANGO_SHELL_CMD=/usr/libexec/kalango/kalango-shell
ENV
d etc/kalango; d etc/systemd/system/multi-user.target.wants
cp crates/kalangod/dist/kalangod.toml "$C/etc/kalango/"
cp crates/kalangod/dist/kalangod.service crates/kalangod/dist/kalango-agentes.slice "$C/etc/systemd/system/"
ln -s ../kalangod.service "$C/etc/systemd/system/multi-user.target.wants/kalangod.service"

# ---- papéis de parede, fontes e os programas que o shell usa (swaybg, wlsunset)
cp wallpapers/sdrustos-16x9-4k.png "$C/usr/share/kalango/papeis-de-parede/padrao.png"
cp wallpapers/sdrustos-16x9-4k.png "$C/usr/share/kalango/papel-de-parede.png"
cp wallpapers/lunar-tall.png "$C/usr/share/kalango/papel-de-parede-alto.png"
cp wallpapers/lunar-wide.png "$C/usr/share/kalango/papel-de-parede-largo.png"
[ -f wallpapers/sdrustos-16x9-4k-claro.png ] && cp wallpapers/sdrustos-16x9-4k-claro.png "$C/usr/share/kalango/papel-de-parede-claro.png"
d usr/local/share/fonts/inter
cp "$DEPS"/usr/share/fonts/opentype/inter/*.otf "$C/usr/local/share/fonts/inter/"
for b in swaybg wlsunset grim; do [ -x "$DEPS/usr/bin/$b" ] && cp "$DEPS/usr/bin/$b" "$C/usr/bin/"; done

# ---- a VM: sem os discos do host, sem o GDM; entra sozinha no tty1 e sobe a sessão KalangoOS
d etc/systemd/system/getty@tty1.service.d; d etc/profile.d; d etc/zsh
echo "# VM do KalangoOS: a raiz é a do host, só leitura, com uma camada descartável" > "$C/etc/fstab"
for u in gdm3.service gdm.service display-manager.service; do ln -s /dev/null "$C/etc/systemd/system/$u"; done
cat > "$C/etc/systemd/system/getty@tty1.service.d/autologin.conf" <<EOF
[Service]
ExecStart=
ExecStart=-/sbin/agetty --autologin ${KALANGO_VM_PESSOA:-$(id -un)} --noclear %I \$TERM
EOF
cat > "$C/etc/profile.d/kalango-vm.sh" <<'SH'
# VM do KalangoOS: o login do tty1 abre a sessão gráfica (uma vez)
if [ "$(tty)" = /dev/tty1 ] && [ -z "${WAYLAND_DISPLAY:-}" ] && [ -z "${KALANGO_SESSAO_ABERTA:-}" ]; then
  export KALANGO_SESSAO_ABERTA=1
  exec /usr/libexec/kalango/kalango-sessao
fi
SH
{ cat /etc/zsh/zprofile 2>/dev/null; echo; echo '. /etc/profile.d/kalango-vm.sh'; } > "$C/etc/zsh/zprofile"
# fotos da tela da sessão, a cada 15 s, numa pasta que o host vê (o QEMU com 3D não tira foto da tela)
d etc/systemd/system/mnt-saida.mount.d
cat > "$C/etc/systemd/system/mnt-saida.mount" <<'UNIT'
[Unit]
Description=KalangoOS VM: pasta compartilhada com o host
[Mount]
What=saida
Where=/mnt/saida
Type=9p
Options=trans=virtio,version=9p2000.L
[Install]
WantedBy=multi-user.target
UNIT
ln -s ../mnt-saida.mount "$C/etc/systemd/system/multi-user.target.wants/mnt-saida.mount"
cat > "$C/etc/systemd/system/kalango-vm-fotos.service" <<'UNIT'
[Unit]
Description=KalangoOS VM: fotos da tela para o host
Requires=mnt-saida.mount
After=mnt-saida.mount
[Service]
ExecStart=/bin/sh -c 'while :; do sleep 15; for s in /run/user/1000/wayland-*; do case "$s" in *.lock) ;; *) XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=$(basename "$s") /usr/bin/grim /mnt/saida/tela.png && break;; esac; done; done'
[Install]
WantedBy=multi-user.target
UNIT
ln -s ../kalango-vm-fotos.service "$C/etc/systemd/system/multi-user.target.wants/kalango-vm-fotos.service"
# os serviços próprios da pessoa (túneis, clusters…) não sobem na VM: ela é só para ver o KalangoOS
H=$(getent passwd "$(id -un)" | cut -d: -f6)
d "${H#/}/.config/systemd/user"
for u in "$H"/.config/systemd/user/*.service "$H"/.config/systemd/user/*.timer; do
  [ -e "$u" ] && ln -sf /dev/null "$C${u}"
done
# o que acontece dentro da VM, no console serial (o registro do QEMU): dispositivos, módulos e os registros da sessão
cat > "$C/etc/systemd/system/kalango-vm-diagnostico.service" <<'UNIT'
[Unit]
Description=KalangoOS VM: diagnóstico no console serial
After=multi-user.target
[Service]
Type=oneshot
ExecStart=/bin/sh -c 'for t in 25 60; do sleep $t; { echo "=== KALANGO-VM diagnóstico ($t s)"; ls -la /dev/dri; lsmod | grep -iE "virtio|drm" ; loginctl list-sessions; journalctl -b --no-pager _COMM=niri | head -80; journalctl -b --no-pager -n 40 _SYSTEMD_USER_UNIT=kalango-shell.service; journalctl -b --no-pager -n 20 -u kalangod; } > /dev/ttyS0 2>&1; done'
[Install]
WantedBy=multi-user.target
UNIT
ln -s ../kalango-vm-diagnostico.service "$C/etc/systemd/system/multi-user.target.wants/kalango-vm-diagnostico.service"
echo "camada pronta em $C ($(du -sh "$C" | cut -f1))"
