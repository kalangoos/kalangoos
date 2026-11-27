#!/bin/bash
# KalangoOS · constrói a imagem do sistema: uma distribuição própria e mínima.
#
# A raiz vem dos pacotes binários do Ubuntu 26.04 LTS — só os de que a pilha do KalangoOS precisa, sem documentação,
# manuais nem traduções além dos nove idiomas do KalangoOS —, com Rust onde já é maduro (coreutils do uutils, sudo-rs, ntpd-rs).
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
#
# A raiz tem duas etapas, e a primeira (os pacotes do Ubuntu, ~9 min) não depende dos nossos binários nem do kernel:
#   KALANGO_IMAGEM_ISO=1 KALANGO_IMAGEM_BASE=raiz.tar imagem/construir.sh base    só a etapa 1: grava raiz.tar
#   KALANGO_IMAGEM_ISO=1 KALANGO_IMAGEM_BASE=raiz.tar imagem/construir.sh         se raiz.tar existe, parte dela (etapa 2:
#                                         o kernel, a camada e os ajustes); sem ele, faz as duas etapas de uma vez
# A CI monta a etapa 1 num job à parte, em paralelo com a compilação, e a guarda em cache.
set -euo pipefail
# (NetworkManager-wait-online.service fica mascarado, no hook mais abaixo: nada na imagem espera "rede no ar" e, num
# computador com porta de cabo sem cabo, ele falhava depois de ~60 s e segurava o boot da sessão ao vivo)
# (what this script makes goes into the system as root's: no group or other write, whatever the builder's own umask
# — with 002 the layer's folders came out 775 and, copied over the base, left /usr and /usr/bin group-writable)
umask 022
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
MODO=${1:-tudo}
BASE=${KALANGO_IMAGEM_BASE:-}
[ "$MODO" = tudo ] || [ "$MODO" = base ] || { echo "uso: imagem/construir.sh [base]"; exit 2; }
[ "$MODO" != base ] || [ -n "$BASE" ] || BASE=$OUT/raiz-base.tar
if [ "$MODO" != base ]; then
for f in target/release/kalango-shell target/release/kalangod target/release/kalango-agente target/release/kalango-aprovacao target/release/kalango-tela \
         target/release/kalango-entrada target/release/kalango-instalador "$DEPS/src/niri/target/release/niri"; do
  [ -x "$f" ] || { echo "falta $f: compile antes (cargo build --release; o niri em poc/.deps/src/niri)"; exit 1; }
done
KDEB=${KERNEL_DEB:-$(ls kernel/out/linux-image-*-kalango_*_amd64.deb 2>/dev/null | grep -v dbg | head -1)}
[ -f "$KDEB" ] || { echo "falta o pacote do kernel em kernel/out (kernel/build.sh)"; exit 1; }
fi

# ---- os pacotes: o que os nossos programas usam (as bibliotecas que eles carregam) e os serviços que as telas controlam
PAC=$(grep -v '^\s*#' imagem/pacotes.txt | tr -s ' \n' ',' | sed 's/^,//; s/,$//')
# (ao vivo: o casper; o instalador o tira do sistema instalado. O initramfs é refeito depois da camada: leva o
# casper.conf do KalangoOS — a conta ao vivo e o nome do computador)
[ "$ISO" = 1 ] && PAC="$PAC,casper"

# (os dois repositórios privados, OPCIONAIS — o que eles trazem está mais abaixo — e a decisão de que entram; os pacotes do
# VirtualDesk precisam estar na raiz de pacotes, que é a etapa 1: por isso a decisão vem antes dela)
VDESK=${KALANGO_VIRTUALDESK_DIR:-$ROOT/../kalango-virtualdesk}
FLUXOS=${KALANGO_FLUXOS_DIR:-$ROOT/../kalango-fluxos}
quer() {  # quer NOME-DA-VARIÁVEL BINÁRIO TEXTO → 0 se deve entrar; 1 se não; sai com erro se foi exigido e falta
  local modo=${!1:-auto}
  [ "${KALANGO_IMAGEM_SEM_APPS:-0}" = 1 ] && modo=0
  case "$modo" in
    0) return 1 ;;
    1) [ -x "$2" ] || { echo "falta $2 ($3): compile o repositório (cargo build --release) ou ponha $1=0" >&2; exit 1; }; return 0 ;;
    *) [ -x "$2" ] && return 0; echo "sem $3 nesta ISO: não achei $2 (compile-o, ou aponte a pasta; $1=1 exige)" >&2; return 1 ;;
  esac
}
vd_quer_pacotes() {  # 0 se a raiz de pacotes leva o que o VirtualDesk precisa (sem exigir o binário: a CI monta a raiz em paralelo)
  local modo=${KALANGO_IMAGEM_VIRTUALDESK:-auto}
  [ "${KALANGO_IMAGEM_SEM_APPS:-0}" = 1 ] && return 1
  case "$modo" in 0) return 1 ;; 1) return 0 ;; *) [ -x "$VDESK/target/release/kalango-virtualdesk" ] ;; esac
}
if vd_quer_pacotes; then
  PAC="$PAC,$(grep -v '^\s*#' imagem/pacotes-virtualdesk.txt | tr -s ' \n' ',' | sed 's/^,//; s/,$//')"
fi

# (o que o mmdebstrap grava e o que deixa de fora: nas duas etapas, e a chave do cache da CI muda com este arquivo)
DPKGOPTS=(
  --dpkgopt='path-exclude=/usr/share/doc/*' --dpkgopt='path-include=/usr/share/doc/*/copyright'
  --dpkgopt='path-exclude=/usr/share/man/*' --dpkgopt='path-exclude=/usr/share/info/*'
  --dpkgopt='path-exclude=/usr/share/locale/*' --dpkgopt='path-include=/usr/share/locale/pt_BR/*'
  --dpkgopt='path-include=/usr/share/locale/pt/*' --dpkgopt='path-include=/usr/share/locale/locale.alias'
  --dpkgopt='path-include=/usr/share/locale/es/*' --dpkgopt='path-include=/usr/share/locale/fr/*'
  --dpkgopt='path-include=/usr/share/locale/it/*' --dpkgopt='path-include=/usr/share/locale/de/*'
  --dpkgopt='path-include=/usr/share/locale/ja/*' --dpkgopt='path-include=/usr/share/locale/ru/*'
  --dpkgopt='path-include=/usr/share/locale/zh_CN/*'
  --dpkgopt='path-exclude=/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc'
  --dpkgopt='path-exclude=/usr/share/fonts/opentype/noto/NotoSerifCJK-*'
)
mmdebstrap() { env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin HOME="$HOME" TMPDIR="$W" /usr/bin/mmdebstrap "$@"; }

# ---- etapa 1: a raiz só com os pacotes (sem nada nosso por cima)
if [ "$MODO" = base ]; then
  rm -f "$BASE"
  mmdebstrap --mode=unshare --variant=minbase --components=main,universe "${DPKGOPTS[@]}" --include="$PAC" "$SUITE" "$BASE" "$MIRROR"
  # (os nós de dispositivo do /dev não se recriam sem root: a etapa 2 deixa o mmdebstrap criar os dele)
  tar --numeric-owner -tvf "$BASE" | awk '$1 ~ /^[cb]/ {print $6}' | xargs -r tar --delete -f "$BASE"
  echo "raiz de pacotes: $BASE ($(du -h --apparent-size "$BASE" | cut -f1))"
  exit 0
fi

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
for b in kalango-shell kalango-aprovacao kalangod kalango-agente kalango-entrada kalango-instalador kalango-tela; do
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
# (o cgroup dos agentes de cada pessoa existe desde o início da sessão: a guarda eBPF o registra antes do primeiro agente)
install -m 644 crates/kalangod/dist/kalango-agentes-usuario.slice "$C/usr/lib/systemd/user/kalango-agentes.slice"
d usr/lib/systemd/user/default.target.wants
ln -s ../kalango-agentes.slice "$C/usr/lib/systemd/user/default.target.wants/kalango-agentes.slice"
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
# (KALANGO_IMAGEM_TODOS_MODELOS=1, só para uma ISO de teste local: leva todos os .gguf da pasta — texto, voz, visão, Omni,
# d1 e os mmproj —, uns 8 GB a mais. A ISO publicada pela CI fica sem modelo nenhum: KALANGO_MODELOS=/nenhum; sem essa variável, só o de texto)
# (KALANGO_IMAGEM_MODELOS="a.gguf b.gguf": exatamente estes arquivos da pasta — o jeito fino; e um arquivo que falta é
# erro, não silêncio. KALANGO_IMAGEM_TODOS_MODELOS=1 leva todos. Sem nenhuma das duas, só o de texto.
# O aviso de licenças vai junto (/usr/share/doc/kalango/modelos/LICENCAS.txt): imagem/licencas-modelos.py confere cada
# arquivo contra o catálogo e se recusa a gerar a ISO de um modelo cuja licença o mantenedor ainda não decidiu.)
# (a ISO publicável NUNCA leva modelo: o LFM3 só é baixado, depois do aceite da licença; embutir é só para ISO local)
if [ "${KALANGO_IMAGEM_PUBLICAVEL:-0}" = 1 ]; then
  [ "${KALANGO_IMAGEM_TODOS_MODELOS:-0}" != 1 ] && [ -z "${KALANGO_IMAGEM_MODELOS:-}" ] || { echo "KALANGO_IMAGEM_PUBLICAVEL=1: uma ISO publicável não leva modelos de IA" >&2; exit 1; }
elif [ "${KALANGO_IMAGEM_TODOS_MODELOS:-0}" = 1 ]; then
  for f in "$MODELOS"/*.gguf; do [ -f "$f" ] && cp "$f" "$C/usr/share/kalango/modelos/"; done
elif [ -n "${KALANGO_IMAGEM_MODELOS:-}" ]; then
  for f in $KALANGO_IMAGEM_MODELOS; do
    [ -f "$MODELOS/$f" ] || { echo "KALANGO_IMAGEM_MODELOS: falta $MODELOS/$f" >&2; exit 1; }
    cp "$MODELOS/$f" "$C/usr/share/kalango/modelos/"
  done
else
  [ -f "$MODELOS/LFM3-1.7B-Q4_K_M.gguf" ] && cp "$MODELOS/LFM3-1.7B-Q4_K_M.gguf" "$C/usr/share/kalango/modelos/"
fi
APROVACAO=$(python3 -I imagem/licencas-modelos.py sessao/dist/share/kalango/loja/modelos.json imagem/licencas-modelos.json \
  "$C/usr/share/kalango/modelos" "$C/usr/share/doc/kalango/modelos/LICENCAS.txt") || exit 1
case "$APROVACAO" in
  NAO-APROVADO*) echo "AVISO: esta imagem leva modelos de IA dentro (${APROVACAO#NAO-APROVADO }): só para uso local, nunca para publicar (docs/ISO-CONTEUDO.md)." >&2 ;;
esac
# ---- GPLv3: a licença e a oferta escrita do código-fonte do que a ISO leva (KalangoOS, niri com os patches, pacotes do
# Ubuntu). O código do KalangoOS e os patches do niri são públicos; a oferta diz onde e por quanto tempo (seção 6.b).
install -d "$C/usr/share/doc/kalango"
install -m 644 LICENSE "$C/usr/share/doc/kalango/LICENSE"
[ -f NOTICE ] && install -m 644 NOTICE "$C/usr/share/doc/kalango/NOTICE"
KOS_COMMIT=$(git rev-parse HEAD 2>/dev/null || echo desconhecido)
NIRI_COMMIT=$(git -C "$DEPS/src/niri" rev-parse HEAD 2>/dev/null || echo desconhecido)
cat > "$C/usr/share/doc/kalango/FONTE.txt" <<FONTE
KalangoOS: código-fonte e oferta escrita (GPLv3, seção 6)

Esta imagem leva software livre sob a GPL-3.0-or-later (LICENSE, nesta pasta) e outras licenças livres.

1. KalangoOS (shell, kalangod, agentes, instalador, tela de entrada, sessão): commit $KOS_COMMIT
   https://github.com/kalangoos/kalangoos  (a mesma versão da ISO está na página de releases, com o arquivo do fonte da marca).
2. niri (compositor) com os patches do KalangoOS: commit $NIRI_COMMIT do niri; os patches estão em sessao/ e poc/ do
   repositório acima. Niri original: https://github.com/YaLTeR/niri (GPL-3.0-or-later).
3. Pacotes do Ubuntu: o código-fonte de cada um sai com "apt source <pacote>" (as fontes deb-src do Ubuntu).
4. Kalango Remote, Kalango Audio, Kalango VirtualDesk: repositórios kalangoos/kalango-remote, kalango-audio e
   kalango-virtualdesk (o fonte do VirtualDesk também vai ao lado da ISO publicada, ver virtualdesk/FONTE.txt).

Se algum desses links sair do ar, peça o código-fonte correspondente em até 3 anos a contar da distribuição desta imagem,
abrindo uma issue no repositório do item 1; ele será enviado em meio físico ou digital pelo custo da mídia.
FONTE
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
  # (o shell que abre o app pelo dock tem LD_PRELOAD=libgtk4-layer-shell: o kalangod recusaria o Remote com ele; o app
  # não precisa dele, e o script tira esse e o LD_AUDIT do ambiente)
  printf '#!/bin/sh\nexec env -u LD_PRELOAD -u LD_AUDIT LD_LIBRARY_PATH=/usr/lib/kalango/remote /usr/lib/kalango/remote/kalango-remote "$@"\n' > "$C/usr/bin/kalango-remote"
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
# ---- os dois repositórios privados (kalangoos/kalango-virtualdesk e kalangoos/kalango-fluxos), OPCIONAIS: entram só
# quando os binários compilados estão ao lado (KALANGO_VIRTUALDESK_DIR / KALANGO_FLUXOS_DIR mudam onde). Sem eles a
# ISO sai como sempre (é o caso da CI de hoje). KALANGO_IMAGEM_VIRTUALDESK / KALANGO_IMAGEM_FLUXOS: auto (padrão: entra
# se estiver compilado), 1 (exige: falha se faltar) ou 0 (nunca). KALANGO_IMAGEM_SEM_APPS=1 também tira os dois.
# O que cada um traz (licenças e tamanho: docs/ISO-CONTEUDO.md, seção 2):
#   VirtualDesk (GPL-3.0-or-later): /usr/bin/kalango-virtualdesk, o atalho, os ícones e as dependências de execução
#     (QEMU, UEFI, TPM: pacotes do Ubuntu, abaixo) mais uma regra do udev que libera o /dev/kvm à pessoa da sessão;
#   Fluxos (Apache-2.0): /usr/lib/kalango/kalango-fluxosd e as unidades de usuário do systemd. O serviço é por ativação
#     de soquete (kalango-fluxos.socket): nada roda até alguém falar com ele, e ele sai sozinho depois de 10 min ocioso.
# KALANGO_IMAGEM_PUBLICAVEL=1 (posto pela CI que publica): a ISO tem de poder ser publicada, e então tudo o que falta
# para isso é erro em vez de aviso: lista de dependências, procedência dos apps, código-fonte da GPL, nenhum modelo.
PUBLICAVEL=${KALANGO_IMAGEM_PUBLICAVEL:-0}
aviso_ou_erro() {  # aviso_ou_erro TEXTO: aviso; na ISO publicável, erro
  echo "$1" >&2
  [ "$PUBLICAVEL" != 1 ] || { echo "(KALANGO_IMAGEM_PUBLICAVEL=1: recusado)" >&2; exit 1; }
}
# (o que cada app que entra compilado deixa na imagem, para quem recebe a ISO e para quem a confere: o texto da licença
# dele, a lista das crates de que foi feito com a licença de cada uma, e o commit de que o binário veio)
inventario() {  # inventario PASTA-DO-APP DESTINO-NA-CAMADA BINÁRIO
  d "$2"
  # (só as crates do alvo do binário: sem --filter-platform, o cargo pede metadados de todas as plataformas, que numa
  # máquina nova não estão baixados e o --offline falha)
  if ! (cd "$1" && cargo metadata --format-version 1 --locked --offline --filter-platform x86_64-unknown-linux-gnu 2>/dev/null) | python3 -I -c '
import json, sys
for p in sorted(json.load(sys.stdin)["packages"], key=lambda p: (p["name"], p["version"])):
    print(p["name"], p["version"], p.get("license") or "(sem licença declarada)")' > "$C/$2/DEPENDENCIAS.txt"; then
    echo "(a lista de dependências não pôde ser gerada nesta construção: cargo metadata falhou)" > "$C/$2/DEPENDENCIAS.txt"
    aviso_ou_erro "$1: sem a lista de dependências (cargo metadata --locked --offline --filter-platform x86_64-unknown-linux-gnu falhou)"
  fi
  for f in LICENSE NOTICE; do [ -f "$1/$f" ] && install -m 644 "$1/$f" "$C/$2/$f"; done
  # (a procedência: o binário vem de qual commit, e a árvore e o binário estão em dia com ele?)
  local commit; commit=$(git -C "$1" rev-parse HEAD 2>/dev/null || echo desconhecido)
  echo "$commit" > "$C/$2/COMMIT"
  echo "$(basename "$1"): commit $commit" >&2
  if [ "$commit" = desconhecido ]; then
    aviso_ou_erro "$1: não é um repositório git: não sei de que commit é o binário"
  else
    [ -z "$(git -C "$1" status --porcelain --untracked-files=no 2>/dev/null)" ] || aviso_ou_erro "$1: a árvore tem mudanças não commitadas: o binário pode não ser o do commit $commit"
    [ "$(stat -c %Y "$3")" -ge "$(git -C "$1" log -1 --format=%ct)" ] || aviso_ou_erro "$3 é mais antigo que o commit $commit: compile de novo"
  fi
  return 0
}
if quer KALANGO_IMAGEM_VIRTUALDESK "$VDESK/target/release/kalango-virtualdesk" "o Kalango VirtualDesk"; then
  install -m 755 "$VDESK/target/release/kalango-virtualdesk" "$C/usr/bin/kalango-virtualdesk"
  sed "s|^Exec=.*|Exec=kalango-virtualdesk|" "$VDESK/dist/os.kalango.VirtualDesk.desktop" > "$C/usr/share/applications/os.kalango.VirtualDesk.desktop"
  for s in 64 128 256 512; do
    d usr/share/icons/hicolor/${s}x${s}/apps
    install -m 644 "$VDESK/assets/icons/hicolor/${s}x${s}/apps/os.kalango.VirtualDesk.png" "$C/usr/share/icons/hicolor/${s}x${s}/apps/"
  done
  # (QEMU para rodar, qemu-utils para os discos, ovmf para o UEFI, swtpm para o TPM: tamanho em docs/ISO-CONTEUDO.md.
  # O /dev/kvm já é da pessoa da sessão pelo 70-uaccess.rules do systemd: nenhuma regra nossa)
  # (os pacotes do VirtualDesk vêm da raiz de pacotes: com a raiz pronta de outra rodada, confira que o QEMU está nela)
  if [ -n "$BASE" ] && [ -f "$BASE" ] && ! tar -tf "$BASE" | grep -q -E '(^|/)usr/bin/qemu-system-x86_64$'; then
    echo "a raiz de pacotes $BASE não tem o QEMU do VirtualDesk: refaça a etapa 1 (imagem/construir.sh base, com o VirtualDesk à vista)" >&2; exit 1
  fi
  inventario "$VDESK" usr/share/doc/kalango/virtualdesk "$VDESK/target/release/kalango-virtualdesk"
  # (a GPL-3.0: quem recebe o binário tem direito ao código-fonte correspondente. Uma ISO publicável só leva o VirtualDesk
  # se o tar.gz do fonte desse commit sai junto, ao lado dela: KALANGO_VIRTUALDESK_FONTE=caminho do arquivo)
  FONTE=${KALANGO_VIRTUALDESK_FONTE:-}
  VD_COMMIT=$(cat "$C/usr/share/doc/kalango/virtualdesk/COMMIT")
  if [ -n "$FONTE" ]; then
    [ -f "$FONTE" ] || { echo "KALANGO_VIRTUALDESK_FONTE: não existe $FONTE" >&2; exit 1; }
    case "$(basename "$FONTE")" in *"$VD_COMMIT"*) ;; *) echo "KALANGO_VIRTUALDESK_FONTE: o nome de $FONTE não tem o commit $VD_COMMIT do binário" >&2; exit 1 ;; esac
    printf 'O Kalango VirtualDesk (/usr/bin/kalango-virtualdesk) é software livre sob a GPL-3.0-or-later (LICENSE, nesta pasta).\nO código-fonte correspondente deste binário (commit %s) é distribuído junto desta ISO, no arquivo\n%s\n(GPLv3, seção 6). Fonte: https://github.com/kalangoos/kalango-virtualdesk\n' "$VD_COMMIT" "$(basename "$FONTE")" > "$C/usr/share/doc/kalango/virtualdesk/FONTE.txt"
  elif [ "$PUBLICAVEL" = 1 ]; then
    echo "o VirtualDesk (GPL-3.0-or-later) só entra numa ISO publicável com o código-fonte do commit $VD_COMMIT ao lado (KALANGO_VIRTUALDESK_FONTE=...tar.gz)" >&2; exit 1
  else
    echo "AVISO: ISO local com o VirtualDesk (GPL-3.0-or-later) sem o código-fonte junto: não a publique (KALANGO_VIRTUALDESK_FONTE)" >&2
  fi
fi
if quer KALANGO_IMAGEM_FLUXOS "$FLUXOS/target/release/kalango-fluxosd" "o serviço dos agentes e fluxos (kalango-fluxosd)"; then
  d usr/lib/kalango; d usr/lib/systemd/user/sockets.target.wants
  install -m 755 "$FLUXOS/target/release/kalango-fluxosd" "$C/usr/lib/kalango/kalango-fluxosd"
  install -m 644 "$FLUXOS/dist/systemd/kalango-fluxos.service" "$FLUXOS/dist/systemd/kalango-fluxos.socket" "$C/usr/lib/systemd/user/"
  # (ativação por soquete: o systemd só abre o soquete; o serviço sobe na primeira conexão, do shell ou do kalangod)
  ln -s ../kalango-fluxos.socket "$C/usr/lib/systemd/user/sockets.target.wants/kalango-fluxos.socket"
  inventario "$FLUXOS" usr/share/doc/kalango/fluxos "$FLUXOS/target/release/kalango-fluxosd"
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

# ---- a raiz
# (and the files copied in, whatever made them: nothing of the system writable by a group or by everyone)
find "$C" ! -type l -perm /022 -exec chmod go-w {} +
rm -f "$OUT/kalangoos.tar"
HOOKS=(
  --customize-hook="copy-in '$W/kernel.deb' /tmp"
  --customize-hook='chroot "$1" dpkg -i /tmp/kernel.deb && rm -f "$1/tmp/kernel.deb"'
  --customize-hook="sync-in '$C' /"
  --customize-hook="cd '$C' && find . -print0 | (cd \"\$1\" && xargs -0 chown -h 0:0)"
  --customize-hook='find "$1/usr/local" ! -type l -perm /022 -exec chmod go-w,g-s {} + ; chgrp -R -h 0 "$1/usr/local"'
  --customize-hook='chroot "$1" useradd --system --create-home --home-dir /var/lib/greeter --shell /usr/sbin/nologin --groups video,render,input greeter || true'
  --customize-hook='printf "%s UTF-8\n" pt_BR.UTF-8 en_US.UTF-8 es_ES.UTF-8 fr_FR.UTF-8 it_IT.UTF-8 de_DE.UTF-8 ja_JP.UTF-8 ru_RU.UTF-8 zh_CN.UTF-8 > "$1/etc/locale.gen" && chroot "$1" locale-gen && printf "LANG=pt_BR.UTF-8\n" > "$1/etc/default/locale"'
  --customize-hook='chroot "$1" sh -c "getent group sudo >/dev/null || groupadd --system sudo"'
  --customize-hook="$CONTA"
  --customize-hook="$SENHA"
  --customize-hook='chroot "$1" systemctl enable greetd.service NetworkManager.service bluetooth.service ntpd-rs.service || true'
  --customize-hook='chroot "$1" systemctl set-default graphical.target'
  --customize-hook='chroot "$1" systemctl mask getty@tty1.service autovt@tty1.service'
  --customize-hook='chroot "$1" systemctl mask NetworkManager-wait-online.service'
  --customize-hook='[ ! -x "$1/usr/bin/glib-compile-schemas" ] || chroot "$1" glib-compile-schemas /usr/share/glib-2.0/schemas'
  --customize-hook='chroot "$1" update-initramfs -u -k all'
  --customize-hook='ln -sf ../usr/lib/os-release "$1/etc/os-release"; rm -f "$1"/etc/update-motd.d/*; rm -f "$1"/etc/legal; : > "$1/etc/motd"'
  # (o mmdebstrap copia o /etc/resolv.conf de quem monta para dentro da raiz — o DNS do computador ou do runner iria na ISO
  # publicada, e na raiz guardada em cache viria de outro run: sem servidores aqui; o NetworkManager grava o arquivo quando a rede liga)
  --customize-hook='rm -f "$1/etc/resolv.conf"; printf "# gravado pelo NetworkManager quando a rede liga\n" > "$1/etc/resolv.conf"'
  --customize-hook="rm -f \"\$1/etc/apt/sources.list\" && printf 'Types: deb\\nURIs: http://archive.ubuntu.com/ubuntu\\nSuites: $SUITE $SUITE-updates $SUITE-backports\\nComponents: main restricted universe multiverse\\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\\n\\nTypes: deb\\nURIs: http://security.ubuntu.com/ubuntu\\nSuites: $SUITE-security\\nComponents: main restricted universe multiverse\\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\\n' > \"\$1/etc/apt/sources.list.d/ubuntu.sources\""
  --customize-hook='rm -rf "$1"/var/cache/apt/archives/*.deb "$1"/var/lib/apt/lists/*'
)
# (etapa 2 quando a CI já montou a raiz de pacotes: parte dela, sem baixar nem instalar nada do Ubuntu; o resultado é o
# mesmo conjunto de arquivos da montagem de uma vez só, que fica como caminho quando não há raiz pronta)
if [ -n "$BASE" ] && [ -f "$BASE" ]; then
  echo ">> partindo da raiz de pacotes $BASE"
  mmdebstrap --mode=unshare --variant=custom --setup-hook="tar-in '$BASE' /" "${DPKGOPTS[@]}" "${HOOKS[@]}" \
    "$SUITE" "$OUT/kalangoos.tar" "$MIRROR"
else
  mmdebstrap --mode=unshare --variant=minbase --components=main,universe "${DPKGOPTS[@]}" --include="$PAC" "${HOOKS[@]}" \
    "$SUITE" "$OUT/kalangoos.tar" "$MIRROR"
fi

# ---- os arquivos do sistema são do root: os nossos entram copiados do computador de quem constrói (uid 1000, que na
# imagem é a conta da pessoa) — uma imagem com /usr da conta deixaria qualquer programa dela trocar o sistema
tar --numeric-owner -tvf "$OUT/kalangoos.tar" | awk '$2 ~ /^1000\// && $6 !~ /^\.\/home\//' > "$W/donos.txt"
if [ -s "$W/donos.txt" ]; then echo "arquivos do sistema com dono errado: $(head -3 "$W/donos.txt" | awk '{print $6}' | tr '\n' ' ')"; exit 1; fi

# ---- toda biblioteca que os nossos programas carregam está na imagem (uma que falte derruba a sessão: o niri sem a
# libseat, na primeira imagem)
. imagem/bibliotecas-chamadas.sh
tar -tf "$OUT/kalangoos.tar" | grep -oE '[^/]+\.so(\.[0-9]+)*$' | sort -u > "$W/sonames.txt"
# (as bibliotecas do llama.cpp vão na pasta dele: contam como presentes)
ls "$C/usr/lib/kalango/llama" "$C/usr/lib/kalango" "$C/usr/lib/kalango/remote" 2>/dev/null | grep -E '\.so' >> "$W/sonames.txt"; sort -u -o "$W/sonames.txt" "$W/sonames.txt"
for b in "$C"/usr/libexec/kalango/*.bin "$C"/usr/libexec/kalango/kalangod "$C"/usr/libexec/kalango/kalango-agente \
         "$C"/usr/libexec/kalango/kalango-entrada "$C"/usr/libexec/kalango/kalango-instalador \
         "$C"/usr/lib/kalango/llama/llama-server "$C"/usr/lib/kalango/llama/*.so "$C"/usr/lib/kalango/moli \
         "$C"/usr/lib/kalango/remote/kalango-remote "$C"/usr/lib/kalango/remote/kalango-remote-core "$C"/usr/bin/kalango-audio \
         "$C"/usr/bin/kalango-virtualdesk "$C"/usr/lib/kalango/kalango-fluxosd; do
  [ -f "$b" ] || continue
  bibliotecas_chamadas "$b"
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
  # (com todos os modelos o squashfs passa de 4 GiB, o limite de um arquivo no ISO 9660: -iso-level 3 grava o
  # arquivo em vários trechos, o que o Linux monta normalmente)
  GRANDE=$W/xorriso-grande; rm -rf "$GRANDE"
  # (o squashfs passa de 4 GiB com todos os modelos ou com uns poucos; a medida é o arquivo, não a variável)
  if [ "$(stat -c %s "$OUT/kalangoos.squashfs")" -gt 4000000000 ]; then
    mkdir -p "$GRANDE"
    printf '#!/bin/sh\ncase "$*" in *-help*) ;; -as*) a=$1; b=$2; shift 2; exec /usr/bin/xorriso -as "$b" -iso-level 3 "$@";; esac\nexec /usr/bin/xorriso "$@"\n' > "$GRANDE/xorriso"; chmod 755 "$GRANDE/xorriso"
  fi
  PATH="$GRANDE:$PATH" grub-mkrescue -o "$OUT/kalangoos.iso" "$I" -- -volid KALANGOOS 2>&1 | grep -v '^xorriso : UPDATE' || true
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
