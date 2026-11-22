#!/bin/bash
# KalangoOS · the shell's memory across apps opened and closed, in an isolated nested session (never your screen).
# Each app of the shell (and each panel) is opened and closed N times; after each app the shell is asked to give the
# heap's free pages back (`kalango-shell memoria aparar`) and its RSS must have come back near where it started: an app
# that keeps something of every open (a dock rebuilt with its old buttons alive, a window hidden with its renderer)
# fails here. A closed app window lets go of its surface and renderer (it is unrealized; its widgets stay) after
# KALANGO_SOLTAR_JANELA_MS (1 s here; 60 s in a real session): sessao/testar-janelas.sh checks that nothing else changes.
# The shell's CPU while idle is measured before and after.
#   ./sessao/testar-memoria.sh [N] [DISPLAY]      (default 20 cycles, display :87)
#   KALANGO_MEMORIA_POR_CICLO_KB: the growth allowed per cycle (default 512 kB) beyond a fixed 16 MB of slack
set -u
cd "$(dirname "$0")/.."
ROOT=$PWD
CICLOS=${1:-20}
DISP=${2:-87}
OUT=$ROOT/target/teste-memoria
rm -rf "$OUT"
mkdir -p "$OUT"
POR_CICLO=${KALANGO_MEMORIA_POR_CICLO_KB:-512}
FOLGA=16
fail=0
ok() { if eval "$2"; then echo "  ok   $1"; else echo "  FALHOU $1"; fail=1; fi; }

KALANGO_SOLTAR_JANELA_MS=1000 KALANGO_ISOLADA=$OUT/iso KALANGO_TESTE_DISPLAY=$DISP ./sessao/isolada.sh 1920x1080 > "$OUT/isolada.log" 2>&1 &
ISO=$!
trap 'kill "$ISO" 2>/dev/null; wait "$ISO" 2>/dev/null' EXIT
for _ in $(seq 150); do [ -s "$OUT/iso/env" ] && break; sleep 2; done
[ -s "$OUT/iso/env" ] || { echo "a sessão isolada não subiu (veja $OUT/isolada.log)"; exit 2; }
. "$OUT/iso/env"
for _ in $(seq 60); do grep -aq "pronto (" "$T/shell.log" && break; sleep 1; done
sleep 8
B=$ROOT/target/${KALANGO_PERFIL:-sessao}/kalango-shell
pid() { pgrep -f "^$B\$" | head -1; }
rss() { awk '/VmRSS/{print int($2/1024)}' "/proc/$(pid)/status"; }
# the shell's CPU time (user + system, in clock ticks)
cpu() { awk '{print $14 + $15}' "/proc/$(pid)/stat"; }
shell_cmd() { "$N" msg action spawn -- "$T/kalango-shell" "$@" >/dev/null; }
ids() { "$N" msg -j windows | python3 -c 'import json,sys; print(" ".join(str(w["id"]) for w in json.load(sys.stdin)))'; }
# the heap's free pages back to the system, then the RSS (MB)
aparado() { local c; c=$(grep -ac "debug memoria:" "$T/shell.log"); shell_cmd memoria aparar 0
  for _ in $(seq 50); do [ "$(grep -ac "debug memoria:" "$T/shell.log")" -gt "$c" ] && break; sleep 0.1; done; rss; }
# a panel closed gives the keyboard back (an overlay left holding it swallows every key after it)
teclado_livre() { local l; l=$("$N" msg -j layers | python3 -c 'import json,sys; print(" ".join(x["namespace"] for x in json.load(sys.stdin) if x.get("keyboard_interactivity") == "Exclusive"))')
  [ -n "$l" ] && echo "  ($1: o teclado ficou com $l)"; }
# one open and close: a window app waits for its window (by app id) and closes it by its niri id, as the person's ×
# does, until it is gone; a panel is toggled twice
janela_de() { "$N" msg -j windows | python3 -c 'import json,sys; print(" ".join(str(w["id"]) for w in json.load(sys.stdin) if w.get("app_id") == sys.argv[1]))' "$1"; }
ciclo() {
  local tipo=$1 id=$2; shift 2
  if [ "$tipo" = painel ]; then shell_cmd "$@"; sleep 1; shell_cmd "$@"; sleep 0.6; teclado_livre "$*"; return; fi
  # Spaces: a second "espacos" goes to the next space (Super+Tab); Esc closes it
  if [ "$tipo" = espacos ]; then shell_cmd espacos; sleep 1; xdotool key --window "$W" Escape; sleep 0.6; teclado_livre espacos; return; fi
  # the print dialog: Ctrl+P in Documents (through the Print portal), then both closed
  if [ "$tipo" = imprimir ]; then
    ciclo_imprimir; return
  fi
  local j=""
  shell_cmd "$@"
  for _ in $(seq 200); do j=$(janela_de "$id"); [ -n "$j" ] && break; sleep 0.1; done
  [ -z "$j" ] && { echo "  ($id não abriu)"; return; }
  sleep 1
  for _ in 1 2 3; do
    for i in $j; do "$N" msg action close-window --id "$i" >/dev/null; done
    for _ in $(seq 30); do [ -z "$(janela_de "$id")" ] && break; sleep 0.1; done
    j=$(janela_de "$id"); [ -z "$j" ] && break
  done
  [ -n "$j" ] && echo "  ($id não fechou)"
  # (Imagens opened without a picture asks for one: the chooser is cancelled, as the person would)
  for i in $(janela_de os.kalango.Seletor); do "$N" msg action close-window --id "$i" >/dev/null; done
  sleep 0.4
}
PDF=$X/area/Documentos/tres-paginas.pdf
mkdir -p "$(dirname "$PDF")"
python3 - "$PDF" <<'PY2'
import sys
from PIL import Image, ImageDraw
pags = []
for i in range(3):
    im = Image.new("RGB", (595, 842), "white"); ImageDraw.Draw(im).text((80, 120), f"Pagina {i + 1}", fill="black"); pags.append(im)
pags[0].save(sys.argv[1], save_all=True, append_images=pags[1:])
PY2
ciclo_imprimir() {
  local j=""
  shell_cmd documentos "$PDF"
  for _ in $(seq 100); do j=$(janela_de os.kalango.Documentos); [ -n "$j" ] && break; sleep 0.1; done
  sleep 1.2
  # (the dialog takes seconds to come through the portal; Ctrl+P again if it did not)
  for _ in 1 2; do
    "$N" msg action focus-window --id "${j%% *}" >/dev/null; sleep 0.4
    xdotool key --window "$W" ctrl+p
    for _ in $(seq 80); do [ -n "$(janela_de os.kalango.Imprimir)" ] && break; sleep 0.1; done
    [ -n "$(janela_de os.kalango.Imprimir)" ] && break
  done
  [ -z "$(janela_de os.kalango.Imprimir)" ] && echo "  (o diálogo de impressão não abriu; em foco: $("$N" msg -j focused-window | python3 -c 'import json,sys; w=json.load(sys.stdin) or {}; print(w.get("app_id"), w.get("title"))'))"
  sleep 1
  for id in os.kalango.Imprimir os.kalango.Documentos; do
    for _ in 1 2 3; do for i in $(janela_de $id); do "$N" msg action close-window --id "$i" >/dev/null; done; sleep 0.5; [ -z "$(janela_de $id)" ] && break; done
  done
}
# idle CPU: the lowest of three minutes with nothing happening (the shell's own minute jobs — energy, suggestions by
# the local model — land in some of them and swing one reading by 30 %; idle work left behind would raise all three)
ocioso() { local a m=999999 d; for _ in 1 2 3; do a=$(cpu); sleep 60; d=$(( $(cpu) - a )); [ "$d" -lt "$m" ] && m=$d; done; echo "$m"; }
# the RSS once what was let go is given back: the lowest of three readings (the last window is unrealized a second after it
# closes and its pages half a second later; one reading alone swung by tens of MB)
assentado() { local m=999999 r; for _ in 1 2 3; do sleep 1.5; r=$(aparado); [ "$r" -lt "$m" ] && m=$r; done; echo "$m"; }

# (kind, app id, the shell's command)
APPS=("janela os.kalango.Calculadora calculadora" "janela os.kalango.Editor editor" "janela os.kalango.Terminal terminal"
  "janela os.kalango.Imagens imagens" "janela os.kalango.Musica musica" "janela os.kalango.Video video"
  "janela os.kalango.Documentos documentos" "janela os.kalango.Discos discos" "janela os.kalango.Apps loja"
  "janela os.kalango.Calendario calendario" "janela os.kalango.Ajuda ajuda" "janela os.kalango.Configuracoes configuracoes"
  "janela os.kalango.Configuracoes configuracoes sistema" "janela os.kalango.Agentes central" "janela os.kalango.Arquivos arquivos"
  "imprimir - -" "painel - privacidade" "painel - foco painel" "espacos - espacos")
VIDEO=$OUT/video.webm
ffmpeg -loglevel error -y -f lavfi -i "testsrc=duration=20:size=320x240:rate=25" -f lavfi -i "sine=frequency=330:duration=20" -c:v libvpx -c:a libvorbis "$VIDEO"
# one pass of everything first: what is built once and kept (caches, the first window of a kind) is the baseline —
# the notices' window too (made once, with the first notice: the disk-space one may come at any minute)
"$N" msg action spawn -- notify-send -a Teste "Memória" "o primeiro aviso cria a janela dos avisos"
# (twice: what is built at the first uses of each app — caches, the allocator's pools — belongs to the baseline;
# measured alone, 20 more cycles of Imagens left nothing, while its first block in the sequence kept ~19 MB)
for _ in 1 2; do
  for a in "${APPS[@]}"; do read -r tipo id cmd <<< "$a"; ciclo "$tipo" "$id" $cmd; done
  # (and a real video: GStreamer's decoders are loaded once per process)
  ciclo janela os.kalango.Video video "$VIDEO"
done
sleep 3
BASE=$(assentado)
# idle CPU after everything was opened twice (the shell's own start-up work is over by then)
CPU0=$(ocioso)
echo "  (base depois de abrir cada app duas vezes: $BASE MB; ${CICLOS} ciclos por app)"
printf '  %-20s %8s %8s %9s %9s\n' app antes "após $CICLOS" aparado "kB/ciclo"
for a in "${APPS[@]}"; do
  read -r tipo id cmd <<< "$a"
  nome=$cmd; [ "$tipo" = imprimir ] && nome=imprimir
  # (KALANGO_MEMORIA_APPS="imprimir arquivos": only these)
  [ -n "${KALANGO_MEMORIA_APPS:-}" ] && [[ " $KALANGO_MEMORIA_APPS " != *" ${nome%% *} "* ]] && continue
  r0=$(assentado)
  for _ in $(seq "$CICLOS"); do ciclo "$tipo" "$id" $cmd; done
  r1=$(rss)
  # (the last window is unrealized a second after it closes, and its pages given back half a second later)
  sleep 2
  r2=$(assentado)
  printf '  %-20s %8s %8s %9s %9s\n' "$nome" "$r0" "$r1" "$r2" $(( (r2 - r0) * 1024 / CICLOS ))
  ok "$nome: aberto e fechado $CICLOS vezes, a memória volta (até $FOLGA MB + $POR_CICLO kB por ciclo)" "[ $((r2 - r0)) -le $((FOLGA + POR_CICLO * CICLOS / 1024)) ]"
done
# every app window used here, closed, let go of its surface and renderer (and is still there, with its widgets)
NAO=""
for a in "${APPS[@]}"; do read -r tipo id cmd <<< "$a"; [ "$tipo" = janela ] || continue
  shell_cmd estado gravar "$id" "$OUT/fim-$id.txt"; sleep 0.5
  head -1 "$OUT/fim-$id.txt" 2>/dev/null | grep -q "realized=false natives=0" || NAO="$NAO ${id#os.kalango.}"
done
echo "  (ainda realizadas depois de fechadas:${NAO:- nenhuma})"
ok "toda janela de app fechada é desrealizada e fica (com os seus widgets)" '[ -z "$NAO" ]'
# a real video: it plays; paused and closed, it lets go of its surface (playing, it would wait)
shell_cmd video "$VIDEO"
for _ in $(seq 60); do grep -aq "debug video: aberto «video.webm»" "$T/shell.log" && break; sleep 0.2; done
sleep 2
focar() { local j; j=$(janela_de "$1"); [ -n "$j" ] && "$N" msg action focus-window --id "${j%% *}" >/dev/null; sleep 0.4; }
focar os.kalango.Video; xdotool key --window "$W" space; sleep 0.8
V0=$(grep -ac "«video.webm» desrealizada" "$T/shell.log")
for i in $(janela_de os.kalango.Video); do "$N" msg action close-window --id "$i" >/dev/null; done
for _ in $(seq 50); do [ "$(grep -ac "«video.webm» desrealizada" "$T/shell.log")" -gt "$V0" ] && break; sleep 0.2; done
ok "Vídeo: um vídeo de verdade abre; pausado e fechado, solta a superfície" \
  'grep -aq "debug video: aberto «video.webm» (320×240" "$T/shell.log" && [ "$(grep -ac "«video.webm» desrealizada" "$T/shell.log")" -gt "$V0" ]'
sleep 2

FIM=$(assentado)
echo "  (memória: base $BASE MB, fim $FIM MB)"
ok "depois de todos os apps, a memória do shell fica perto da base" "[ $((FIM - BASE)) -le $((FOLGA * 2 + POR_CICLO * CICLOS / 1024)) ]"
# (last: a Music with a track is kept, as it should be — after the total above)
# Music playing behind its closed window goes on while the window lets go of its surface: closed, it pauses; MPRIS (the
# island, the lock screen) plays
# it on; well past the delay it still plays and its MPRIS name is still there
BUS=$(tr '\0' '\n' < "/proc/$(pid)/environ" | sed -n 's/^DBUS_SESSION_BUS_ADDRESS=//p')
mpris() { DBUS_SESSION_BUS_ADDRESS=$BUS gdbus call --session --dest org.mpris.MediaPlayer2.kalango_musica --object-path /org/mpris/MediaPlayer2 "$@" 2>&1; }
FAIXA=$OUT/faixa.flac
ffmpeg -loglevel error -y -f lavfi -i "sine=frequency=440:duration=120" -metadata title="Faixa de teste" -metadata artist="Teste" "$FAIXA"
shell_cmd musica "$FAIXA"
for _ in $(seq 60); do grep -aq "debug musica: tocando «Faixa de teste»" "$T/shell.log" && break; sleep 0.2; done
sleep 1
M0=$(grep -ac "«Música» desrealizada" "$T/shell.log")
for i in $(janela_de os.kalango.Musica); do "$N" msg action close-window --id "$i" >/dev/null; done
sleep 1
mpris --method org.mpris.MediaPlayer2.Player.PlayPause >/dev/null
sleep 4
ESTADO=$(mpris --method org.freedesktop.DBus.Properties.Get org.mpris.MediaPlayer2.Player PlaybackStatus)
M1=$(grep -ac "«Música» desrealizada" "$T/shell.log")
echo "  (música atrás da janela fechada, 3 s depois do prazo: $ESTADO; a janela foi desrealizada $M1 vez(es) na rodada)"
ok "Música: tocando atrás da janela fechada (pelo MPRIS), segue tocando com a janela desrealizada" 'echo "$ESTADO" | grep -q "Playing" && [ "$M1" -gt "${M0:-0}" ]'
mpris --method org.mpris.MediaPlayer2.Player.Pause >/dev/null

CPU1=$(ocioso)
echo "  (CPU parado, o menor de três minutos: $CPU0 ticks no começo, $CPU1 no fim)"
# (within 15 %, plus 3 ticks for the reading's own grain)
ok "o shell parado não gasta mais CPU depois de abrir e fechar os apps (até 15 % a mais)" "[ $CPU1 -le $((CPU0 * 115 / 100 + 3)) ]"
exit $fail
