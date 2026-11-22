#!/bin/bash
# KalangoOS · a window closed and opened again shows what it showed, as it always did — whatever the shell does with it
# while it is closed (it lets it go after KALANGO_SOLTAR_JANELA_MS) — in an isolated nested session (never your
# screen). For each app: something of the person's is put in its window (an expression in the Calculator, a question
# in Help, a chosen drive in Disks, a document scrolled and zoomed, a command's output in the Terminal…), the window's
# state is written down (`kalango-shell estado gravar`), the window is closed, the delay passes, the app is opened
# again and its state written down again. Two passes in the same session: one with the windows kept hidden for ever
# (`estado prazo` very long: what the shell always did), one with them let go after 1.5 s. What changes across a close
# only when the window is let go is a regression; so is a window that was not let go (still realized after the delay).
# Lines that change by themselves (a clock, a meter) are found by writing the state twice before closing.
#   ./sessao/testar-janelas.sh [DISPLAY] [APPS…]     (default :88; every app)
set -u
cd "$(dirname "$0")/.."
ROOT=$PWD
DISP=${1:-88}
shift || true
OUT=$ROOT/target/teste-janelas
rm -rf "$OUT"
mkdir -p "$OUT"
fail=0
ok() { if eval "$2"; then echo "  ok   $1"; else echo "  FALHOU $1"; fail=1; fi; }

# (the session has no microphone: the test recorder "records" a file and waits to be stopped, as pw-record does)
ffmpeg -loglevel error -y -f lavfi -i "sine=frequency=300:duration=2" "$OUT/fala.wav"
export KALANGO_GRAVADOR=$ROOT/sessao/ferramentas/gravador-teste.sh KALANGO_TESTE_FALA=$OUT/fala.wav
KALANGO_SOLTAR_JANELA_MS=1500 KALANGO_ISOLADA=$OUT/iso KALANGO_TESTE_DISPLAY=$DISP ./sessao/isolada.sh 1920x1080 > "$OUT/isolada.log" 2>&1 &
ISO=$!
trap 'kill "$ISO" 2>/dev/null; wait "$ISO" 2>/dev/null' EXIT
for _ in $(seq 150); do [ -s "$OUT/iso/env" ] && break; sleep 2; done
[ -s "$OUT/iso/env" ] || { echo "a sessão isolada não subiu (veja $OUT/isolada.log)"; exit 2; }
. "$OUT/iso/env"
for _ in $(seq 60); do grep -aq "pronto (" "$T/shell.log" && break; sleep 1; done
sleep 8
B=$ROOT/target/${KALANGO_PERFIL:-sessao}/kalango-shell
shell_cmd() { "$N" msg action spawn -- "$T/kalango-shell" "$@" >/dev/null; }
janela_de() { "$N" msg -j windows | python3 -c 'import json,sys; print(" ".join(str(w["id"]) for w in json.load(sys.stdin) if w.get("app_id") == sys.argv[1]))' "$1"; }
focar() { local j; j=$(janela_de "$1"); [ -n "$j" ] && "$N" msg action focus-window --id "${j%% *}" >/dev/null; sleep 0.4; }
# the state of APP's window to FILE (and wait for it)
gravar() { local c; c=$(grep -ac "debug estado: $1 gravado em $2" "$T/shell.log"); shell_cmd estado gravar "$1" "$2"
  for _ in $(seq 50); do [ "$(grep -ac "debug estado: $1 gravado em $2" "$T/shell.log")" -gt "$c" ] && return 0; sleep 0.1; done; echo "  (estado de $1 não gravado)"; }
mexer() { shell_cmd estado "$@"; sleep 0.6; }
# the path of the Nth (from 0) visible widget whose dump line matches REGEX
achar() { python3 - "$1" "$2" "${3:-0}" <<'PY'
import re, sys
f, rx, n = sys.argv[1], re.compile(sys.argv[2]), int(sys.argv[3])
hits = [l.split(' ')[0] for l in open(f) if not l.startswith('WINDOW') and ' v=1 ' in l and rx.search(l)]
print(hits[n] if len(hits) > n else '')
PY
}
fechar() { for _ in 1 2 3; do for i in $(janela_de "$1"); do "$N" msg action close-window --id "$i" >/dev/null; done; sleep 0.8; [ -z "$(janela_de "$1")" ] && return 0; done
  echo "  ($1 não fechou)"; }
esperar_janela() { for _ in $(seq 150); do [ -n "$(janela_de "$1")" ] && return 0; sleep 0.1; done; echo "  ($1 não abriu)"; }
BUS=$(tr '\0' '\n' < "/proc/$(pgrep -f "^$B\$" | head -1)/environ" | sed -n 's/^DBUS_SESSION_BUS_ADDRESS=//p')
mpris() { DBUS_SESSION_BUS_ADDRESS=$BUS gdbus call --session --dest org.mpris.MediaPlayer2.kalango_musica --object-path /org/mpris/MediaPlayer2 "$@" >/dev/null 2>&1; }

# the person's files for the apps (in the test's own folders)
DOCS=$X/area/Documentos; mkdir -p "$DOCS"
python3 - "$DOCS/cinco-paginas.pdf" <<'PY'
import sys
from PIL import Image, ImageDraw
p = []
for i in range(5):
    im = Image.new("RGB", (595, 842), "white"); ImageDraw.Draw(im).text((80, 120), f"Pagina {i + 1}", fill="black"); p.append(im)
p[0].save(sys.argv[1], save_all=True, append_images=p[1:])
PY
ffmpeg -loglevel error -y -f lavfi -i "sine=frequency=440:duration=300" -metadata title="Faixa de teste" -metadata artist="Teste" "$X/area/faixa.flac"
ffmpeg -loglevel error -y -f lavfi -i "testsrc=duration=120:size=320x240:rate=25" -f lavfi -i "sine=frequency=330:duration=120" -c:v libvpx -c:a libvorbis "$X/area/video.webm"

# ---- each app: its id, how it is opened again (as the person would: from the dock, without a file), and what of
# the person's is put in its window before it is closed
declare -A ID=( [calculadora]=Calculadora [ajuda]=Ajuda [discos]=Discos [loja]=Apps [calendario]=Calendario [musica]=Musica
  [video]=Video [imagens]=Imagens [arquivos]=Arquivos [central]=Agentes [configuracoes]=Configuracoes [documentos]=Documentos
  [editor]=Editor [terminal]=Terminal [monitor]=Monitor )
# (state setters: the window is open and written to $1; they change it as the person would)
preparar() {
  local app=$1 f=$2 id=os.kalango.${ID[$1]} p
  case $app in
    calculadora)
      focar "$id"; xdotool type --window "$W" --delay 60 "12+30"; xdotool key --window "$W" Return; sleep 0.4
      mexer ativar "$id" "$(achar "$f" 'GtkToggleButton .*cfg-seg' 1)"; sleep 0.4
      focar "$id"; xdotool type --window "$W" --delay 60 "7*6"; xdotool key --window "$W" Return; sleep 0.4
      mexer ativar "$id" "$(achar "$f" 'GtkToggleButton .*cfg-seg' 2)"; sleep 0.4
      focar "$id"; xdotool type --window "$W" --delay 60 "100";;
    ajuda) mexer texto "$id" "$(achar "$f" 'GtkText ')" "como conectar o wi-fi";;
    discos) mexer ativar "$id" "$(achar "$f" 'GtkButton .*dsc-unidade' 2)"; mexer texto "$id" "$(achar "$f" 'GtkText ')" "clonar o disco";;
    loja) mexer texto "$id" "$(achar "$f" 'GtkText ')" "editor de imagens";;
    calendario) mexer texto "$id" "$(achar "$f" 'GtkText ')" "dentista quinta 14h";;
    musica) mpris --method org.mpris.MediaPlayer2.Player.Pause; sleep 0.8;;
    # (space plays and pauses: paused once, on the first pass, it stays paused)
    video) [ "$PASSO" = 1 ] && { focar "$id"; xdotool key --window "$W" space; sleep 0.8; };;
    imagens) focar "$id"; xdotool key --window "$W" Right; sleep 1;;
    arquivos) focar "$id"; xdotool key --window "$W" ctrl+t; sleep 1; xdotool key --window "$W" Down; sleep 0.5;;
    central) focar "$id"; xdotool type --window "$W" --delay 40 "rascunho de teste";;
    configuracoes) mexer texto "$id" "$(achar "$f" 'GtkText ' 1)" "nome digitado";;
    documentos)
      p=$(python3 -c '
import re,sys
best=("",0)
for l in open(sys.argv[1]):
    m=re.search(r"^(\S+) GtkScrolledWindow v=1 .* v=\d+/(\d+)",l)
    if m and int(m.group(2))>best[1]: best=(m.group(1),int(m.group(2)))
print(best[0])' "$f")
      mexer rolar "$id" "$p" 900; focar "$id"; xdotool key --window "$W" ctrl+plus; sleep 0.8;;
    editor) focar "$id"; xdotool key --window "$W" ctrl+End; sleep 0.3;;
    terminal) focar "$id"; xdotool type --window "$W" --delay 30 "echo marcador-da-saida"; xdotool key --window "$W" Return; sleep 1;;
    monitor) :;;
  esac
  sleep 0.8
}
abrir_primeiro() {
  case $1 in
    musica) shell_cmd musica "$X/area/faixa.flac";;
    video) shell_cmd video "$X/area/video.webm";;
    imagens) shell_cmd imagens "$A/viagem-praia.png";;
    arquivos) shell_cmd arquivos "$A";;
    central) shell_cmd central conversas;;
    configuracoes) shell_cmd configuracoes sistema;;
    documentos) shell_cmd documentos "$DOCS/cinco-paginas.pdf";;
    editor) shell_cmd editor "$A/notas.md";;
    *) shell_cmd "$1";;
  esac
}
# one pass of one app: $3 = prefix of the files
passar() {
  local app=$1 pre=$2 id=os.kalango.${ID[$1]}
  [ -n "$(janela_de "$id")" ] || { abrir_primeiro "$app"; esperar_janela "$id"; sleep 3; }
  gravar "$id" "$pre-0.txt"
  preparar "$app" "$pre-0.txt"
  # (one window resized by the person before it closes: the size it comes back with is compared too)
  [ "$app" = editor ] && { focar "$id"; "$N" msg action set-window-width 1100 >/dev/null; sleep 1; }
  gravar "$id" "$pre-a.txt"; sleep 1.5; gravar "$id" "$pre-a2.txt"
  fechar "$id"
  sleep 4
  gravar "$id" "$pre-h.txt"
  # reopened: at once (~300 ms: the first frames, what a person sees first) and once settled
  shell_cmd "$app"; esperar_janela "$id"; sleep 0.3
  gravar "$id" "$pre-c.txt"; sleep 2.5
  gravar "$id" "$pre-b.txt"
}
# the paths that changed across the close (in A vs B, not in A vs A2)
# (`só-pessoa`: only what the person sets — a field's text, a page, a toggle, a choice, the scroll, the focus —: the
# Monitor's numbers and process list change all the time by themselves)
# (the window's own line too: title, size, maximized, focus…; not how many natives, the popovers open at the time)
mudou() { python3 - "$1-a.txt" "$1-a2.txt" "$1-${3:-b}.txt" "${2:-}" <<'PY'
import re, sys
only_person = sys.argv[4] == 'só-pessoa'
def load(f):
    d = {}
    for l in open(f):
        if l.startswith('WINDOW'):
            h = re.sub(r' natives=\d+', '', l.rstrip('\n'))
            if only_person:
                h = re.sub(r' widgets=\d+', '', h)
            d['WINDOW'] = h
            continue
        p = l.split(' ')[0]
        l = l.rstrip('\n')
        if only_person:
            if not re.search(r'text=|page=|active=|selected=|FOCUS| v=\d+/', l):
                continue
            l = re.sub(r' label=".*?"', '', l)
            l = re.sub(r'(v=\d+)/\d+ h=(\d+)/\d+', r'\1 h=\2', l)
            l = re.sub(r' \[[^]]*\]', '', l)
        d[p] = l
    return d
a, a2, b = (load(f) for f in sys.argv[1:4])
vol = {p for p in set(a) | set(a2) if a.get(p) != a2.get(p)}
for p in sorted(set(a) | set(b)):
    if p not in vol and a.get(p) != b.get(p):
        print(p)
PY
}

APPS=("${@:-calculadora ajuda discos loja calendario musica video imagens arquivos central configuracoes documentos editor terminal monitor}")
read -r -a APPS <<< "${APPS[*]}"
for app in "${APPS[@]}"; do
  id=os.kalango.${ID[$app]}
  echo "== $app"
  shell_cmd estado prazo 4294967295
  PASSO=1 passar "$app" "$OUT/$app-escondida"
  shell_cmd estado prazo 1500
  PASSO=2 passar "$app" "$OUT/$app-solta"
  modo=""; [ "$app" = monitor ] && modo=só-pessoa
  { mudou "$OUT/$app-escondida" "$modo"; mudou "$OUT/$app-escondida" "$modo" c | sed 's/$/@300ms/'; } > "$OUT/$app-escondida.mudou"
  { mudou "$OUT/$app-solta" "$modo"; mudou "$OUT/$app-solta" "$modo" c | sed 's/$/@300ms/'; } > "$OUT/$app-solta.mudou"
  REG=$(python3 -c 'import sys; h=set(open(sys.argv[1]).read().split()); print("\n".join(p for p in open(sys.argv[2]).read().split() if p not in h))' "$OUT/$app-escondida.mudou" "$OUT/$app-solta.mudou")
  if [ -n "$REG" ]; then
    echo "  (muda só quando solta: $(echo "$REG" | wc -l) widgets)"
    for p in $(echo "$REG" | head -6); do
      q=${p%@300ms}; f=b; [ "$q" != "$p" ] && f=c
      echo "    antes: $(grep -a "^$q " "$OUT/$app-solta-a.txt" | cut -c1-150)"
      echo "    depois ($f): $(grep -a "^$q " "$OUT/$app-solta-$f.txt" | cut -c1-150)"
    done
  fi
  H=$(head -1 "$OUT/$app-solta-h.txt")
  echo "  (fechada, depois do prazo: ${H#WINDOW $id })"
  ok "$app: fechada, é desrealizada (nenhuma superfície) e a janela fica, com os seus widgets" 'echo "$H" | grep -q "realized=false natives=0"'
  ok "$app: fechar e reabrir mostra o que mostrava, como quando a janela ficava escondida" '[ -z "$REG" ]'
  fechar "$id"
done
# Help closed while it listens: the listening stops (the recorder is gone, the microphone is not open behind a closed
# window); opened again, the button is back to normal and the card says why
if [[ " ${APPS[*]} " == *" ajuda "* ]]; then
  echo "== ajuda ouvindo"
  shell_cmd ajuda; esperar_janela os.kalango.Ajuda; sleep 2
  gravar os.kalango.Ajuda "$OUT/ajuda-mic-0.txt"
  O0=$(grep -ac "debug ajuda: ouvindo" "$T/shell.log")
  mexer ativar os.kalango.Ajuda "$(achar "$OUT/ajuda-mic-0.txt" 'GtkButton .*aju-mic')"
  for _ in $(seq 30); do [ "$(grep -ac "debug ajuda: ouvindo" "$T/shell.log")" -gt "$O0" ] && break; sleep 0.1; done
  gravador() { pgrep -f "^/bin/bash $KALANGO_GRAVADOR " >/dev/null && echo sim || echo não; }
  G1=$(gravador)
  fechar os.kalango.Ajuda; sleep 2.5
  G2=$(gravador)
  gravar os.kalango.Ajuda "$OUT/ajuda-mic-h.txt"
  shell_cmd ajuda; esperar_janela os.kalango.Ajuda; sleep 2
  gravar os.kalango.Ajuda "$OUT/ajuda-mic-1.txt"
  echo "  (gravando ao abrir o microfone: $G1; com a janela fechada e solta: $G2)"
  ok "ajuda: fechada enquanto ouve, a escuta para (o gravador some, o microfone não fica aberto) e o botão volta ao normal" \
    '[ "$G1" = sim ] && [ "$G2" = não ] && head -1 "$OUT/ajuda-mic-h.txt" | grep -q "realized=false natives=0" && ! grep -a "aju-mic" "$OUT/ajuda-mic-1.txt" | grep -q gravando'
  fechar os.kalango.Ajuda
fi
# Music paused by MPRIS (a media key, the island) while its window is closed and unrealized: opened again, its button
# shows "play" from the first frames (~300 ms), as with the timer that never slept
if [[ " ${APPS[*]} " == *" musica "* ]]; then
  echo "== musica pausada fechada"
  shell_cmd estado prazo 1500
  abrir_primeiro musica; esperar_janela os.kalango.Musica; sleep 3
  mpris --method org.mpris.MediaPlayer2.Player.Pause; sleep 0.8
  gravar os.kalango.Musica "$OUT/mus-pausada.txt"
  mpris --method org.mpris.MediaPlayer2.Player.Play; sleep 0.8
  gravar os.kalango.Musica "$OUT/mus-tocando.txt"
  fechar os.kalango.Musica; sleep 4
  mpris --method org.mpris.MediaPlayer2.Player.Pause; sleep 1
  gravar os.kalango.Musica "$OUT/mus-h.txt"
  shell_cmd musica; esperar_janela os.kalango.Musica; sleep 0.3
  gravar os.kalango.Musica "$OUT/mus-c.txt"
  botao() { grep -a -A2 "GtkButton .*\.mus-tocar" "$1" | grep -ao "texture=[0-9a-fx#]*" | head -1; }
  # (the hidden window's button already, before it maps: what its first frame draws, however fast the reopen)
  P=$(botao "$OUT/mus-pausada.txt"); TC=$(botao "$OUT/mus-tocando.txt"); HB=$(botao "$OUT/mus-h.txt"); C=$(botao "$OUT/mus-c.txt")
  echo "  (botão pausada: $P; tocando: $TC; fechada depois da pausa: $HB; reaberta em 300 ms: $C)"
  ok "musica: pausada pelo MPRIS com a janela fechada e solta, reabre mostrando tocar" \
    '[ -n "$P" ] && [ "$P" != "$TC" ] && [ "$HB" = "$P" ] && [ "$C" = "$P" ] && head -1 "$OUT/mus-h.txt" | grep -q "realized=false natives=0"'
  fechar os.kalango.Musica
fi
grep -a "CRITICAL" "$T/shell.log" | grep -v keyval | sed 's/^.*CRITICAL \*\*: [0-9:.]* //' | sort | uniq -c | sort -rn | head -5 | sed 's/^/  (crítico: /; s/$/)/'
exit $fail
