#!/bin/bash
# KalangoOS · the screens at several screen sizes, without touching your screen.
#
# Starts the session (nested niri, as rodar.sh) in a virtual X display with throwaway profile folders and stand-ins for
# the system's tools (as testar.sh), then for each screen size opens the system's windows (Arquivos in its four views,
# every section of Configurações, every page of the agents center), maximized and half a screen wide, and runs
# `kalango-shell auditar-layout`: a part made to a fixed width that grew with something inside it, or a part past its
# window's edges, is a failure. Every step leaves a capture in target/tamanhos/ to look at.
#
#   ./sessao/tamanhos.sh                 # all the sizes
#   KALANGO_TAMANHOS="1366x768 1920x1080" ./sessao/tamanhos.sh
set -u
cd "$(dirname "$0")/.."
ROOT=$PWD
DISP=:${KALANGO_TESTE_DISPLAY:-78}
OUT=$(mktemp -d /tmp/kalango-tamanhos-XXXX)
CAP="$ROOT/target/tamanhos"
rm -rf "$CAP"; mkdir -p "$CAP"
N="${KALANGO_NIRI:-$ROOT/poc/.deps/src/niri/target/release/niri}"
TAMANHOS=${KALANGO_TAMANHOS:-"1280x720 1366x768 1600x900 1920x1080 2560x1440 3440x1440"}
Xvfb "$DISP" -screen 0 3440x1440x24 >/dev/null 2>&1 &
XP=$!
sleep 1
X="$OUT/perfil"
mkdir -p "$X/data" "$X/config/kalango" "$X/state" "$X/cache" "$X/area" "$OUT/power_supply/BAT0" "$OUT/lid/LID0"
echo 100 > "$OUT/power_supply/BAT0/charge_control_end_threshold"
printf '{"screen_off_min": 0, "suspend_min": 0, "notifications": {"dnd_auto": false}}' > "$X/config/kalango/configuracoes.json"
# a folder with what the views must hold: long names, many kinds, folders, copies, pictures
A="$X/area/Downloads"
python3 - "$A" <<'PY'
import os, sys, shutil, struct, zlib, time
d = sys.argv[1]; os.makedirs(d, exist_ok=True)
for p in ["PDFs", "Imagens", "Instaladores", "Compactados", "uma pasta com um nome bem comprido para ver como corta"]:
    os.makedirs(os.path.join(d, p), exist_ok=True)
def png(p, cor, w=64, h=48):
    ch = lambda t, b: struct.pack('>I', len(b)) + t + b + struct.pack('>I', zlib.crc32(t + b) & 0xffffffff)
    raw = b''.join(b'\0' + bytes(cor) * w for _ in range(h))
    open(p, 'wb').write(b'\x89PNG\r\n\x1a\n' + ch(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)) + ch(b'IDAT', zlib.compress(raw)) + ch(b'IEND', b''))
agora = time.time()
for i, (n, c) in enumerate([("viagem-praia.png", (40, 120, 160)), ("foto(1).png", (90, 60, 120)), ("noite.png", (10, 12, 30)), ("lua.png", (200, 200, 210))]):
    png(os.path.join(d, n), c, 320, 200); os.utime(os.path.join(d, n), (agora - i * 86400 * 3,) * 2)
for n in ["relatorio-out.txt", "notas.md", "setup-kalangoos.deb", "Fernanda Fernandes - Dubladores e Seus Personagens (versão final revisada).txt",
          "a_arte_da_guerra_50s_sem_espacos_no_nome_do_arquivo_mesmo.txt", "fatura.txt"]:
    open(os.path.join(d, n), "w").write("conteúdo de teste\n" * 20)
shutil.copy(os.path.join(d, "fatura.txt"), os.path.join(d, "fatura-copia.txt"))
PY
env DISPLAY="$DISP" WAYLAND_DISPLAY= XMODIFIERS=@im=none KALANGO_SHELL_DEBUG=1 KALANGO_MEMORIA_CHAVEIRO=nao KALANGO_COFRE=arquivo KALANGO_AREA_TESTE="$X/area" KALANGO_PACKAGEKIT_BUS=session \
    KALANGO_NMCLI="$ROOT/sessao/ferramentas/nmcli-teste.py" KALANGO_NMCLI_ESTADO="$OUT/nmcli.json" KALANGO_BLUETOOTHCTL="$ROOT/sessao/ferramentas/bluetoothctl-teste.py" KALANGO_BLUETOOTHCTL_ESTADO="$OUT/bluetoothctl.json" \
    KALANGO_PW_DUMP="$ROOT/sessao/ferramentas/pw-dump-teste" KALANGO_WPCTL="$ROOT/sessao/ferramentas/wpctl-teste" KALANGO_PW_CLI="$ROOT/sessao/ferramentas/pw-cli-teste" KALANGO_PW_RECORD="$ROOT/sessao/ferramentas/pw-record-teste" \
    KALANGO_PW_ESTADO="$OUT/pipewire.json" KALANGO_PW_FILTRO="$ROOT/sessao/ferramentas/pipewire-teste" KALANGO_SONS=nao KALANGO_POWERPROFILESCTL="$ROOT/sessao/ferramentas/powerprofilesctl-teste" \
    KALANGO_UPOWER="$ROOT/sessao/ferramentas/upower-teste" KALANGO_ENERGIA_ESTADO="$OUT/energia.json" KALANGO_POWER_SUPPLY="$OUT/power_supply" KALANGO_ACPI_LID="$OUT/lid" KALANGO_MINUTO_MS=60000 \
    KALANGO_CLIMA_URL=http://127.0.0.1:1 XDG_DATA_HOME="$X/data" XDG_CONFIG_HOME="$X/config" XDG_STATE_HOME="$X/state" XDG_CACHE_HOME="$X/cache" \
    __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json LIBGL_ALWAYS_SOFTWARE=1 \
    ./sessao/rodar.sh > "$OUT/rodar.log" 2>&1 &
for _ in $(seq 90); do grep -q "sessão de teste em" "$OUT/rodar.log" && break; sleep 1; done
T=$(grep -o "/tmp/kalango-sessao-[A-Za-z0-9]*" "$OUT/rodar.log" | head -1)
for _ in $(seq 90); do grep -q pronto "$T/shell.log" 2>/dev/null && break; sleep 1; done
SOCK=$(grep -ao "niri.wayland-[0-9]*\.[0-9]*\.sock" "$T/niri.log" | head -1)
export NIRI_SOCKET="$XDG_RUNTIME_DIR/$SOCK" DISPLAY="$DISP"
W=$(xdotool search --name "^niri$" | head -1)
xdotool windowfocus --sync "$W"
sleep 1
falhas=0
relatorio="$CAP/relatorio.txt"
: > "$relatorio"
janelas() { "$N" msg -j windows | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))'; }
# audits what is open now; $1 names the step (and its capture)
auditar() {
  local n0; n0=$(grep -ac "debug layout: .* janelas, " "$T/shell.log")
  "$N" msg action spawn -- "$T/kalango-shell" auditar-layout
  for _ in $(seq 30); do [ "$(grep -ac "debug layout: .* janelas, " "$T/shell.log")" -gt "$n0" ] && break; sleep 0.2; done
  local fim; fim=$(grep -a "debug layout: .* janelas, " "$T/shell.log" | tail -1)
  local p; p=$(echo "$fim" | grep -o "[0-9]* problemas" | grep -o "[0-9]*")
  "$N" msg action screenshot-screen --path "$CAP/$1.png" >/dev/null 2>&1
  if [ "${p:-1}" != 0 ]; then
    falhas=$((falhas + 1))
    echo "FALHOU $1: $fim" | tee -a "$relatorio"
    grep -a "debug layout: \[" "$T/shell.log" | tail -n "${p:-5}" | sed 's/^/    /' | tee -a "$relatorio"
  else
    echo "  ok   $1" | tee -a "$relatorio"
  fi
}
# the focused window: full screen, then half a screen wide
larguras() { # nome
  "$N" msg action set-column-width "100%"; "$N" msg action reset-window-height; sleep 1.2; auditar "$1-cheia"
  "$N" msg action set-column-width "50%"; sleep 1.2; auditar "$1-metade"
}
fechar() { while [ "$(janelas)" -gt 0 ]; do "$N" msg action close-window; sleep 0.6; done; }
for tam in $TAMANHOS; do
  w=${tam%x*}; h=${tam#*x}
  xdotool windowsize "$W" "$w" "$h"
  sleep 2.5
  echo "== $tam" | tee -a "$relatorio"
  # Arquivos: the list, the icons, the columns, the gallery
  "$N" msg action spawn -- "$T/kalango-shell" arquivos "$A"
  for _ in $(seq 40); do [ "$(janelas)" -ge 1 ] && break; sleep 0.3; done
  sleep 1.5
  for v in 2 1 3 4; do
    xdotool key --window "$W" "ctrl+$v"; sleep 0.8
    larguras "$tam-arquivos-vista$v"
  done
  fechar
  # Configurações: each section
  for s in inicio rede bluetooth tela som energia notificacoes entrada aparencia agentes privacidade acessibilidade sistema; do
    "$N" msg action spawn -- "$T/kalango-shell" configuracoes "$s"
    for _ in $(seq 40); do [ "$(janelas)" -ge 1 ] && break; sleep 0.3; done
    sleep 1.5
    larguras "$tam-configuracoes-$s"
  done
  fechar
  # the agents center: each page
  for p in conversas agentes rotinas linha memoria permissoes uso ajustes; do
    "$N" msg action spawn -- "$T/kalango-shell" central "$p"
    for _ in $(seq 40); do [ "$(janelas)" -ge 1 ] && break; sleep 0.3; done
    sleep 1.5
    larguras "$tam-central-$p"
  done
  fechar
done
echo "$falhas passos com problemas (capturas e relatório em $CAP)"
"$N" msg action quit --skip-confirmation >/dev/null 2>&1
sleep 2
kill "$XP" 2>/dev/null
pkill -P $$ 2>/dev/null
exit $((falhas > 0))
