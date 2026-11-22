#!/bin/bash
# KalangoOS · modo privacidade do controle remoto (patch 0012 do niri), num niri aninhado e isolado:
#   com uma sessão remota que ninguém aprovou neste computador (linha "pid ... privacidade" do arquivo de permissões),
#   o monitor local mostra uma cobertura preta com só os sinais do Kalango Remote por cima (namespace kalango-remote-*),
#   enquanto a captura de quem controla (wlr-screencopy) continua vendo a área de trabalho de verdade.
#
#   1. sem a palavra, nada cobre: a tela local e a captura mostram a área de trabalho;
#   2. com a palavra: a tela local fica preta, o aviso (kalango-remote-banner) aparece por cima, outra camada
#      (overlay de outro programa) fica escondida; a captura remota vê a área de trabalho;
#   3. a entrada: o clique e a tecla da pessoa não chegam à janela escondida, o clique remoto (ponteiro virtual) chega;
#      Esc Esc continua rodando KALANGO_ESC_DUPLO e o botão de parar fica por cima;
#   4. quando o núcleo morre, a cobertura sai sozinha; quando a linha sai do arquivo, também;
#   5. a linha de quem não é o dono do arquivo (gravável por outros) não cobre nada;
#   6. o app (dono do aviso) morre e o núcleo continua vivo: a cobertura sai (ninguém ficaria para mostrar o aviso e o
#      botão Parar); sem o aviso aberto também não há cobertura;
#   7. um programa que não é o app e usa o namespace kalango-remote-banner não é desenhado por cima, não recebe cliques
#      e não liga a cobertura; outra camada overlay acima do aviso não engole o clique no botão Parar (o aviso o recebe).
#   (Não coberto aqui: a tela de bloqueio — o niri aninhado não bloqueia sem um cliente ext-session-lock — nem o
#   quadro de transição de tela, que a cobertura também não desenha.)
#
#   KALANGO_NIRI=caminho/do/niri  KALANGO_WLR_POC=caminho/do/wlr-remote-poc  ./sessao/testar-privacidade.sh [DISPLAY]
# (o niri com o patch 0012; o wlr-remote-poc é kalangoos/poc/wlr-remote do kalango-remote). Nada toca a sua sessão
# nem a sua pasta pessoal: o niri roda num Xvfb próprio, o arquivo de permissões é temporário.
set -u
cd "$(dirname "$0")/.."
DISP=${1:-:127}
N=${KALANGO_NIRI:?KALANGO_NIRI: o niri com o patch 0012}
POC=${KALANGO_WLR_POC:?KALANGO_WLR_POC: o wlr-remote-poc}
T=$(mktemp -d /tmp/kalango-privacidade-XXXX)
FALHAS=0
ok() { echo "ok   $*"; }
falha() { echo "FALHA $*"; FALHAS=$((FALHAS + 1)); }
PIDS=()
limpar() {
    for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null; done
    sleep 0.5
    rm -rf "$T"
}
trap limpar EXIT

# o arquivo de permissões: do dono (esta pessoa, nas sessões de desenvolvimento) e sem escrita para os outros
CONTROLE=$T/controle-remoto
EXE=$(readlink -f "$POC")
publicar() { printf '%s' "$1" > "$CONTROLE.tmp"; chmod 644 "$CONTROLE.tmp"; mv "$CONTROLE.tmp" "$CONTROLE"; }
publicar "exe $EXE captura,ponteiro,teclado
"

cat > "$T/niri.kdl" <<KDL
output "winit" {
    mode "1280x800"
}
hotkey-overlay {
    skip-at-startup
}
prefer-no-csd
binds {
    Mod+Q { close-window; }
}
KDL
Xvfb "$DISP" -screen 0 1280x1000x24 >/dev/null 2>&1 &
PIDS+=($!)
sleep 1
env -u WAYLAND_DISPLAY DISPLAY="$DISP" __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json LIBGL_ALWAYS_SOFTWARE=1 \
    KALANGO_NIRI_CONTROLE="$CONTROLE" KALANGO_NIRI_CONTROLE_DONO="$(id -u)" KALANGO_ESC_DUPLO="touch $T/esc-duplo" \
    dbus-run-session -- "$N" --config "$T/niri.kdl" > "$T/niri.log" 2>&1 &
PIDS+=($!)
SOCK=
for _ in $(seq 60); do
    SOCK=$(sed 's/\x1b\[[0-9;]*m//g' "$T/niri.log" | grep -o "listening on Wayland socket: [^ ]*" | awk '{print $NF}')
    [ -n "$SOCK" ] && break
    sleep 0.5
done
[ -n "$SOCK" ] || { echo "o niri não abriu"; tail -5 "$T/niri.log"; exit 1; }
W=$(DISPLAY="$DISP" xdotool search --name "^niri$" | head -1)
[ -n "$W" ] && DISPLAY="$DISP" xdotool windowmove "$W" 0 0 windowsize "$W" 1280 800
sleep 1
export WAYLAND_DISPLAY=$SOCK

# a área de trabalho: uma janela vermelha em tela cheia que registra o clique e as teclas
cat > "$T/area.py" <<'PY'
import gi
gi.require_version("Gtk", "4.0")
from gi.repository import Gio, Gtk
app = Gtk.Application(application_id="os.kalango.TestePrivacidade", flags=Gio.ApplicationFlags.NON_UNIQUE)
def ativar(a):
    w = Gtk.ApplicationWindow(application=a, title="area")
    css = Gtk.CssProvider()
    css.load_from_string("window, box, button { background: #ff0000; background-image: none; border: none; }")
    Gtk.StyleContext.add_provider_for_display(w.get_display(), css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
    b = Gtk.Button()
    b.set_hexpand(True); b.set_vexpand(True)
    b.connect("clicked", lambda _b: print("CLIQUE", flush=True))
    kc = Gtk.EventControllerKey()
    kc.connect("key-pressed", lambda c, v, code, st: print("TECLA", v, flush=True) or False)
    kc.connect("key-released", lambda c, v, code, st: print("SOLTA", v, flush=True))
    w.add_controller(kc)
    w.set_child(b)
    w.fullscreen()
    w.present()
app.connect("activate", ativar)
app.run(None)
PY
GDK_BACKEND=wayland /usr/bin/python3 "$T/area.py" > "$T/area.out" 2>&1 &
PIDS+=($!)
# (espera a janela aparecer: o primeiro quadro vem devagar com o renderizador por software)
for _ in $(seq 40); do grep -q . "$T/area.out" 2>/dev/null; DISPLAY="$DISP" ffmpeg -loglevel error -y -f x11grab -video_size 1280x800 -i "$DISP" -frames:v 1 "$T/espera.png" 2>/dev/null && python3 -c "
import sys
from PIL import Image
sys.exit(0 if Image.open('$T/espera.png').convert('RGB').getpixel((900,600))[0] > 200 else 1)" && break; sleep 1; done
# o app e o núcleo do Kalango Remote, de mentira mas com a mesma relação: o app é uma cópia do wlr-remote-poc que se chama
# kalango-remote e abre o aviso (camada kalango-remote-banner) e o núcleo é o filho dele, uma cópia do sleep que se chama
# kalango-remote-core ao lado (o niri confere que o pai do núcleo é o kalango-remote ao lado dele)
mkdir -p "$T/bin" "$T/outro"
cp "$POC" "$T/bin/kalango-remote"
cp "$(command -v sleep)" "$T/bin/kalango-remote-core"
cp "$POC" "$T/outro/impostor"
inicio_de() { python3 - "$1" <<'PY'
import sys
s = open(f"/proc/{sys.argv[1]}/stat").read()
print(s.rsplit(")", 1)[1].split()[19])
PY
}
abrir_app() { # abre o app (o aviso verde) e o núcleo filho; define APP, NUCLEO e INICIO
  "$T/bin/kalango-remote" layer kalango-remote-banner 0 255 0 "$T/bin/kalango-remote-core" 3000 > "$T/banner.out" 2>&1 &
  APP=$!
  PIDS+=($APP)
  for _ in $(seq 30); do grep -q "^child" "$T/banner.out" 2>/dev/null && break; sleep 0.2; done
  NUCLEO=$(awk '/^child/ {print $2}' "$T/banner.out")
  INICIO=$(inicio_de "$NUCLEO")
}

# cor de um ponto da tela local (o que o monitor mostra: a janela X do niri aninhado) e da captura remota
local_px() { # x y
    DISPLAY="$DISP" ffmpeg -loglevel error -y -f x11grab -video_size 1280x800 -i "$DISP" -frames:v 1 "$T/local.png" 2>/dev/null
    python3 - "$T/local.png" "$1" "$2" <<'PY'
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("RGB")
print("%d,%d,%d" % im.getpixel((int(sys.argv[2]), int(sys.argv[3]))))
PY
}
remote_px() { # x y
    "$POC" capture "$T/remota.png" > /dev/null 2>&1 || { echo "falhou"; return; }
    python3 - "$T/remota.png" "$1" "$2" <<'PY'
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("RGB")
print("%d,%d,%d" % im.getpixel((int(sys.argv[2]), int(sys.argv[3]))))
PY
}
is_red() { case "$1" in 2[45][0-9],[0-9],[0-9]|2[45][0-9],[0-9][0-9],[0-9][0-9]|25[0-5],[0-9],[0-9]) return 0 ;; esac; python3 -c "import sys; r,g,b=map(int,'$1'.split(',')); sys.exit(0 if r>200 and g<60 and b<60 else 1)" 2>/dev/null; }
is_black() { python3 -c "import sys; r,g,b=map(int,'$1'.split(',')); sys.exit(0 if r<8 and g<8 and b<8 else 1)" 2>/dev/null; }
is_green() { python3 -c "import sys; r,g,b=map(int,'$1'.split(',')); sys.exit(0 if g>200 and r<60 and b<60 else 1)" 2>/dev/null; }
settle() { sleep 1.5; }
# vivo de verdade: um processo que morreu e o pai ainda não recolheu (zumbi) não conta
vivo() { [ -r "/proc/$1/stat" ] && ! sed 's/.*) //' "/proc/$1/stat" | grep -q '^Z'; }

DISPLAY="$DISP" xdotool windowfocus "$W" mousemove 300 400
# 1. sem a palavra
L=$(local_px 900 600); R=$(remote_px 900 600)
is_red "$L" && ok "sem privacidade: a tela local mostra a área de trabalho ($L)" || falha "sem privacidade: a tela local deveria ser vermelha, é $L"
is_red "$R" && ok "sem privacidade: a captura remota mostra a área de trabalho ($R)" || falha "sem privacidade: a captura remota deveria ser vermelha, é $R"

# (controle: sem a cobertura a tecla e o clique da pessoa chegam)
: > "$T/area.out"
DISPLAY="$DISP" xdotool key a
sleep 0.7
grep -q TECLA "$T/area.out" && ok "sem privacidade: a tecla da pessoa chega à janela" || falha "sem privacidade: a tecla da pessoa deveria chegar (o teste do teclado não vale sem isso)"
: > "$T/area.out"

# o app (com o aviso verde) e o núcleo dele; outro programa abre o overlay azul e, depois, um impostor amarelo com o
# mesmo namespace do aviso, os dois acima do aviso (criados depois)
abrir_app
"$T/outro/impostor" layer outro-overlay 0 0 255 > "$T/outro.out" 2>&1 &
PIDS+=($!)
sleep 1
"$T/outro/impostor" layer kalango-remote-banner 255 255 0 > "$T/impostor.out" 2>&1 &
IMPOSTOR=$!
PIDS+=($IMPOSTOR)
sleep 2

# uma tecla segurada quando a cobertura começa (a soltura tem de chegar ao programa escondido)
: > "$T/area.out"
DISPLAY="$DISP" xdotool keydown a
sleep 0.5

# 2. com a palavra
publicar "exe $EXE captura,ponteiro,teclado
pid $NUCLEO $INICIO captura,ponteiro,teclado,privacidade
"
settle
DISPLAY="$DISP" xdotool keyup a
sleep 0.5
grep -q "SOLTA" "$T/area.out" && ok "a tecla segurada quando a cobertura começou foi solta no programa escondido" || falha "a soltura da tecla segurada não chegou ao programa escondido (tecla presa)"
: > "$T/area.out"
L_BAIXO=$(local_px 900 600)
L_AVISO=$(local_px 640 30)
is_black "$L_BAIXO" && ok "com privacidade: a tela local está preta ($L_BAIXO)" || falha "com privacidade: a tela local deveria ser preta, é $L_BAIXO"
is_green "$L_AVISO" && ok "com privacidade: só o aviso do app aparece por cima ($L_AVISO): nem o overlay azul de outro programa nem o impostor amarelo com o mesmo namespace" || falha "com privacidade: o ponto do aviso deveria ser verde (o do app), é $L_AVISO"
R=$(remote_px 900 600)
is_red "$R" && ok "com privacidade: a captura remota ainda vê a área de trabalho ($R)" || falha "com privacidade: a captura remota deveria ser vermelha, é $R"

# o clique no botão Parar (o aviso) chega ao aviso do app mesmo com outras camadas overlay acima dele
: > "$T/banner.out.depois"
CLIQUES_ANTES=$(grep -c BUTTON "$T/banner.out")
DISPLAY="$DISP" xdotool mousemove 630 25 click 1
sleep 0.7
[ "$(grep -c BUTTON "$T/banner.out")" -gt "$CLIQUES_ANTES" ] && ok "o clique da pessoa no aviso chega ao aviso do app, com um overlay e um impostor acima dele" || falha "o clique no aviso não chegou ao aviso do app"
if grep -q BUTTON "$T/outro.out" "$T/impostor.out"; then falha "o clique foi parar numa camada que não é o app ($(cat "$T/outro.out" "$T/impostor.out" | grep -c BUTTON))"; else ok "nem o overlay de outro programa nem o impostor recebem o clique"; fi

# 3. a entrada
: > "$T/area.out"
DISPLAY="$DISP" xdotool mousemove 300 400 click 1
sleep 0.5
DISPLAY="$DISP" xdotool key a
sleep 0.5
# (um atalho do compositor que age às cegas, Alt+Q fecha a janela, não faz nada sob a cobertura)
DISPLAY="$DISP" xdotool key alt+q
sleep 0.5
if grep -q "CLIQUE\|TECLA\|SOLTA" "$T/area.out"; then falha "a entrada da pessoa chegou à janela escondida: $(cat "$T/area.out")"; else ok "o clique e a tecla da pessoa não chegam à janela escondida"; fi
"$POC" click 300 400 > /dev/null 2>&1
sleep 0.7
grep -q "CLIQUE" "$T/area.out" && ok "o clique de quem controla (ponteiro virtual) chega à janela" || falha "o clique remoto deveria chegar à janela"
rm -f "$T/esc-duplo"
DISPLAY="$DISP" xdotool key Escape
sleep 0.15
DISPLAY="$DISP" xdotool key Escape
sleep 0.7
[ -f "$T/esc-duplo" ] && ok "Esc Esc roda KALANGO_ESC_DUPLO com a cobertura ligada" || falha "Esc Esc não rodou com a cobertura ligada"
# (e o compositor mesmo tira a cobertura e para o núcleo: a saída não depende do app)
sleep 1.5
vivo "$NUCLEO" && falha "o Esc Esc sob a cobertura não parou o núcleo" || ok "o Esc Esc sob a cobertura parou o núcleo"
L=$(local_px 900 600)
is_red "$L" && ok "e a cobertura saiu ($L)" || falha "o Esc Esc sob a cobertura não tirou a cobertura, a tela é $L"
kill "$APP" 2>/dev/null; wait "$APP" 2>/dev/null
abrir_app
publicar "exe $EXE captura,ponteiro,teclado
pid $NUCLEO $INICIO captura,ponteiro,teclado,privacidade
"
settle

# 4. o app morre e o núcleo continua: a cobertura sai
kill -9 "$APP"; wait "$APP" 2>/dev/null
kill -0 "$NUCLEO" 2>/dev/null && ok "(o núcleo continua vivo depois da morte do app)" || falha "o teste esperava o núcleo vivo"
settle
L=$(local_px 900 600)
is_red "$L" && ok "o app morreu com o núcleo vivo: a cobertura saiu ($L)" || falha "o app morreu e a tela local deveria voltar, é $L"
kill "$NUCLEO" 2>/dev/null; wait "$NUCLEO" 2>/dev/null

# 5. o núcleo morre com o app vivo: a cobertura sai
abrir_app
publicar "exe $EXE captura,ponteiro,teclado
pid $NUCLEO $INICIO captura,ponteiro,teclado,privacidade
"
settle
L=$(local_px 900 600)
is_black "$L" && ok "(app e núcleo novos: a cobertura voltou)" || falha "a cobertura deveria voltar com o app novo, a tela local é $L"
kill "$NUCLEO"; wait "$NUCLEO" 2>/dev/null
settle
L=$(local_px 900 600)
is_red "$L" && ok "o núcleo morreu: a cobertura saiu sozinha ($L)" || falha "o núcleo morreu e a tela local deveria voltar, é $L"
kill "$APP" 2>/dev/null; wait "$APP" 2>/dev/null

# 6. sem o aviso aberto (um núcleo de verdade com o app vivo, mas nenhuma camada kalango-remote-banner dele) não há cobertura
"$T/bin/kalango-remote" layer kalango-remote-outline 0 255 0 "$T/bin/kalango-remote-core" 3000 > "$T/semaviso.out" 2>&1 &
SA=$!
PIDS+=($SA)
for _ in $(seq 30); do grep -q "^child" "$T/semaviso.out" 2>/dev/null && break; sleep 0.2; done
NUCLEO=$(awk '/^child/ {print $2}' "$T/semaviso.out")
publicar "exe $EXE captura,ponteiro,teclado
pid $NUCLEO $(inicio_de "$NUCLEO") captura,ponteiro,teclado,privacidade
"
settle
L=$(local_px 900 600)
is_red "$L" && ok "sem o aviso (kalango-remote-banner) do app aberto, não há cobertura ($L)" || falha "sem o aviso a tela local não deveria ser coberta, é $L"
kill "$NUCLEO" "$SA" 2>/dev/null; wait "$SA" 2>/dev/null

# 7. a linha tirada do arquivo, com o app e o aviso no ar
abrir_app
publicar "exe $EXE captura,ponteiro,teclado
pid $NUCLEO $INICIO captura,ponteiro,teclado,privacidade
"
settle
L=$(local_px 900 600)
is_black "$L" || falha "a cobertura deveria voltar com a nova sessão, a tela local é $L"
publicar "exe $EXE captura,ponteiro,teclado
"
settle
L=$(local_px 900 600)
is_red "$L" && ok "a linha saiu do arquivo: a cobertura saiu ($L)" || falha "a linha saiu do arquivo e a tela local deveria voltar, é $L"
: > "$T/area.out"
DISPLAY="$DISP" xdotool mousemove 300 400 click 1
sleep 0.5
grep -q CLIQUE "$T/area.out" && ok "sem a cobertura, o clique da pessoa chega de novo" || falha "sem a cobertura o clique da pessoa deveria chegar"

# 7b. a janela escondida continua aberta: o Alt+Q sob a cobertura não a fechou
L=$(local_px 900 600)
is_red "$L" && ok "o atalho Alt+Q (close-window) não fechou a janela escondida" || falha "a janela escondida foi fechada por um atalho às cegas ($L)"

# 7c. o app trava (SIGSTOP; o aviso continua mapeado) com a cobertura ligada: Esc Esc no teclado desta máquina tira a
# cobertura e para o núcleo, sem depender do app
publicar "exe $EXE captura,ponteiro,teclado
pid $NUCLEO $INICIO captura,ponteiro,teclado,privacidade
"
settle
L=$(local_px 900 600)
is_black "$L" || falha "a cobertura deveria estar ligada antes de travar o app, a tela é $L"
kill -STOP "$APP"
rm -f "$T/esc-duplo"
DISPLAY="$DISP" xdotool key Escape
sleep 0.15
DISPLAY="$DISP" xdotool key Escape
sleep 2
L=$(local_px 900 600)
is_red "$L" && ok "o app travou: Esc Esc tirou a cobertura sem depender dele ($L)" || falha "o app travou e Esc Esc não tirou a cobertura, a tela é $L"
vivo "$NUCLEO" && falha "o núcleo continuou vivo depois do Esc Esc sob a cobertura" || ok "o Esc Esc também parou o núcleo (a sessão acabou)"
kill -CONT "$APP" 2>/dev/null; kill "$APP" 2>/dev/null; wait "$APP" 2>/dev/null
abrir_app

# 8. um arquivo gravável por outros não vale
publicar "pid $NUCLEO $INICIO captura,ponteiro,teclado,privacidade
"
chmod 666 "$CONTROLE"
settle
L=$(local_px 900 600)
is_red "$L" && ok "arquivo gravável por outros: não cobre nada ($L)" || falha "arquivo gravável por outros não deveria cobrir, a tela local é $L"
grep -a "cannot be read or trusted\|not owned" "$T/niri.log" > /dev/null && ok "(o niri avisou no registro que não confia no arquivo)" || echo "(sem aviso no registro: $(tail -1 "$T/niri.log" | cut -c1-80))"

[ -n "${KALANGO_MANTER:-}" ] && { cp "$T"/*.png /tmp/ 2>/dev/null; cp "$T/niri.log" /tmp/claude-1000/niri-mut.log; }
echo "registros em $T (apagados ao sair)"
[ "$FALHAS" -eq 0 ] && echo "TUDO CERTO" || echo "$FALHAS FALHA(S)"
exit "$FALHAS"
