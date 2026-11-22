#!/bin/bash
# KalangoOS · an isolated session to look at (and drive) by hand or by script, without touching your screen: the same
# virtual display, throwaway profile and stand-ins as tamanhos.sh. Leaves its variables in $OUT/env (source it) and
# a "Downloads" folder of samples; ends when this script is killed.
#   ./sessao/isolada.sh [LARGURAxALTURA]   (default 1920x1080)
set -u
cd "$(dirname "$0")/.."
ROOT=$PWD
DISP=:${KALANGO_TESTE_DISPLAY:-79}
OUT=${KALANGO_ISOLADA:-$(mktemp -d /tmp/kalango-isolada-XXXX)}
mkdir -p "$OUT"
N="${KALANGO_NIRI:-$ROOT/poc/.deps/src/niri/target/release/niri}"
TAM=${1:-1920x1080}
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
env XDG_CURRENT_DESKTOP=KalangoOS XDG_DESKTOP_PORTAL_DIR="$ROOT/sessao/dist/share/xdg-desktop-portal/portals" XDG_CONFIG_DIRS="$ROOT/sessao/dist/etc/xdg:/etc/xdg" GDK_DEBUG=portals GTK_USE_PORTAL=1 DISPLAY="$DISP" WAYLAND_DISPLAY= XMODIFIERS=@im=none KALANGO_SHELL_DEBUG=1 KALANGO_MEMORIA_CHAVEIRO=nao KALANGO_COFRE=arquivo KALANGO_AREA_TESTE="$X/area" KALANGO_PACKAGEKIT_BUS=session \
    KALANGO_NMCLI="$ROOT/sessao/ferramentas/nmcli-teste.py" KALANGO_NMCLI_ESTADO="$OUT/nmcli.json" KALANGO_BLUETOOTHCTL="$ROOT/sessao/ferramentas/bluetoothctl-teste.py" KALANGO_BLUETOOTHCTL_ESTADO="$OUT/bluetoothctl.json" \
    KALANGO_MODELOS="${XDG_DATA_HOME:-$HOME/.local/share}/kalango/modelos" KALANGO_GST="$ROOT/sessao/ferramentas/gst-teste" KALANGO_GST_LOG="$OUT/gst.log" KALANGO_CAPTURA_DESTINO="$OUT/capturas" KALANGO_PW_DUMP="$ROOT/sessao/ferramentas/pw-dump-teste" KALANGO_WPCTL="$ROOT/sessao/ferramentas/wpctl-teste" KALANGO_PW_CLI="$ROOT/sessao/ferramentas/pw-cli-teste" KALANGO_PW_RECORD="$ROOT/sessao/ferramentas/pw-record-teste" \
    KALANGO_PW_ESTADO="$OUT/pipewire.json" KALANGO_PW_FILTRO="$ROOT/sessao/ferramentas/pipewire-teste" KALANGO_SONS=nao KALANGO_POWERPROFILESCTL="$ROOT/sessao/ferramentas/powerprofilesctl-teste" \
    KALANGO_UPOWER="$ROOT/sessao/ferramentas/upower-teste" KALANGO_ENERGIA_ESTADO="$OUT/energia.json" KALANGO_POWER_SUPPLY="$OUT/power_supply" KALANGO_ACPI_LID="$OUT/lid" KALANGO_MINUTO_MS=${KALANGO_MINUTO_MS:-60000} \
    KALANGO_DISCOS_TESTE="$OUT/discos.json" KALANGO_DISCOS_LOG="$OUT/discos.log" KALANGO_DISCOS_TESTE_STATS="$OUT/discos.stats" KALANGO_LPSTAT="$ROOT/sessao/ferramentas/lpstat-teste" KALANGO_LP="$ROOT/sessao/ferramentas/lp-teste" KALANGO_LP_LOG="$OUT/impressao.log" \
    KALANGO_CLIMA_URL=http://127.0.0.1:1 RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" HOME="$X/area" XDG_DATA_HOME="$X/data" XDG_CONFIG_HOME="$X/config" XDG_STATE_HOME="$X/state" XDG_CACHE_HOME="$X/cache" \
    __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json LIBGL_ALWAYS_SOFTWARE=1 \
    ./sessao/rodar.sh > "$OUT/rodar.log" 2>&1 &
# (a loaded machine can take minutes to build and start it)
for _ in $(seq 300); do grep -q "sessão de teste em" "$OUT/rodar.log" && break; sleep 1; done
T=$(grep -o "/tmp/kalango-sessao-[A-Za-z0-9]*" "$OUT/rodar.log" | head -1)
for _ in $(seq 300); do grep -q pronto "$T/shell.log" 2>/dev/null && break; sleep 1; done
SOCK=$(grep -ao "niri.wayland-[0-9]*\.[0-9]*\.sock" "$T/niri.log" | head -1)
export NIRI_SOCKET="$XDG_RUNTIME_DIR/$SOCK" DISPLAY="$DISP"
W=$(xdotool search --name "^niri$" | head -1)
xdotool windowfocus --sync "$W"
sleep 1
xdotool windowsize "$W" "${TAM%x*}" "${TAM#*x}"
sleep 2
printf 'export T=%s NIRI_SOCKET=%s DISPLAY=%s W=%s N=%s A=%s X=%s\n' "$T" "$NIRI_SOCKET" "$DISP" "$W" "$N" "$A" "$X" > "$OUT/env"
echo "sessão isolada pronta: source $OUT/env"
trap '"$N" msg action quit --skip-confirmation >/dev/null 2>&1; sleep 1; kill "$XP" 2>/dev/null; pkill -P $$ 2>/dev/null' EXIT
while :; do sleep 60; done
