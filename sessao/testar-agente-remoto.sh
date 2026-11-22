#!/bin/bash
# KalangoOS · o agente propõe conectar ("conecte no computador da minha mãe…") e quem decide é um clique no painel do plano
# do Kalango Remote de verdade, num Xvfb próprio: o modelo real (LFM3) chama remoto.conectar, o painel aparece no app,
# este script clica em "Aprovar e conectar" (e depois em "Cancelar") e o agente só diz que está conectando depois do
# primeiro. Precisa do app compilado (KALANGO_REMOTE_REPO = o repositório kalango-remote, com target/debug/kalango-remote) e
# do kalangod e do modelo como em crates/kalango-agente/tests/motor.rs.
#   KALANGO_REMOTE_REPO=../kalango-remote ./sessao/testar-agente-remoto.sh [DISPLAY]      (padrão :137)
set -u
cd "$(dirname "$0")/.."
REPO=$(realpath "${KALANGO_REMOTE_REPO:?KALANGO_REMOTE_REPO: o repositório kalango-remote compilado}")
DISP=${1:-:137}
T=$(mktemp -d /tmp/kalango-agente-remoto-XXXX)
Xvfb "$DISP" -screen 0 1600x1000x24 > /dev/null 2>&1 &
XP=$!
sleep 1
# (KALANGO_DEPS: o poc/.deps do KalangoOS, com o gtk4-layer-shell, quando o repositório do app não está ao lado dele)
export KALANGO_DEPS=${KALANGO_DEPS:-$PWD/poc/.deps}
export DISPLAY=$DISP GDK_BACKEND=x11 KALANGO_REMOTE_DEMO=$T/perfil LANGUAGE=pt_BR
cat > "$T/app.sh" <<SH
#!/bin/sh
cd "$REPO" && exec kalangoos/scripts/rodar-app.sh --demo "\$@"
SH
chmod +x "$T/app.sh"
"$T/app.sh" > "$T/app.log" 2>&1 &
AP=$!
CL=
trap 'kill $CL $AP $XP 2>/dev/null; pkill -P $AP 2>/dev/null; echo "registros em $T"' EXIT
WID=
for _ in $(seq 30); do WID=$(xdotool search --onlyvisible --name "Kalango Remote" | head -1); [ -n "$WID" ] && break; sleep 1; done
[ -n "$WID" ] || { echo "o app não abriu"; tail -3 "$T/app.log"; exit 1; }
sleep 2
xdotool windowmove "$WID" 0 0 windowsize "$WID" 1500 1000
sleep 1
# o clicador: procura o botão de destaque ("Aprovar e conectar") na região do painel e clica nele (ou no "Cancelar", 398 px à
# esquerda do centro do primeiro), conforme $T/clicar
clicar() {
  while [ ! -f "$T/fim" ]; do
    sleep 1
    [ -f "$T/clicar" ] || continue
    ffmpeg -loglevel error -y -f x11grab -video_size 1600x1000 -i "$DISP" -frames:v 1 "$T/tela.png" 2>/dev/null || continue
    P=$(python3 -W ignore - "$T/tela.png" <<'PY'
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("RGB")
# a cor de destaque do botão primário (lilás): bloco grande de pixels dessa cor à direita
pts = [(x, y) for x in range(1290, 1466, 2) for y in range(100, 560, 2) if (lambda p: abs(p[0]-165) < 18 and abs(p[1]-161) < 18 and abs(p[2]-226) < 20)(im.getpixel((x, y)))]
if len(pts) > 400:
    # (o botão é o maior bloco dessa cor: a etiqueta "aprovar" do passo e o "Limpar" do fundo são pequenos)
    best = max({p[1] // 20 for p in pts}, key=lambda r: sum(1 for p in pts if p[1] // 20 == r))
    pts = [p for p in pts if abs(p[1] // 20 - best) <= 1]
    xs = [p[0] for p in pts]; ys = [p[1] for p in pts]
    print((min(xs)+max(xs))//2, (min(ys)+max(ys))//2)
PY
)
    [ -n "$P" ] || continue
    set -- $P
    if [ "$(cat "$T/clicar")" = cancelar ]; then X=$(( $1 - 398 )); else X=$1; fi
    xdotool mousemove "$X" "$2" click 1
    echo "clicou $(cat "$T/clicar") em $X,$2" >> "$T/cliques.log"
    sleep 3   # (o painel fecha; a vontade de clicar fica até o teste acabar: o modelo pode propor de novo)
  done
}
clicar &
CL=$!
export KALANGO_PAINEL_REAL=$T/app.sh PKG_CONFIG_PATH="$PWD/poc/.deps/root/usr/lib/x86_64-linux-gnu/pkgconfig"
FALHAS=0
for decisao in approved cancelled; do
  [ "$decisao" = approved ] && echo aprovar > "$T/clicar" || echo cancelar > "$T/clicar"
  rm -f "$T/cliques.log"
  if KALANGO_PAINEL_ESPERADO=$decisao cargo test -q -p kalango-agente --test remote -- --ignored --nocapture a_click_on_the_real_panel_decides > "$T/teste-$decisao.log" 2>&1; then
    [ -s "$T/cliques.log" ] || { echo "FALHA o teste passou sem nenhum clique no painel ($decisao)"; FALHAS=$((FALHAS + 1)); continue; }
    echo "ok   um clique ($([ "$decisao" = approved ] && echo "Aprovar e conectar" || echo Cancelar)) no painel real: o agente foi avisado como deveria ($(tail -1 "$T/cliques.log"))"
  else
    echo "FALHA com o clique para $decisao"; grep -a "panicked\|assert" "$T/teste-$decisao.log" | head -3; FALHAS=$((FALHAS + 1))
  fi
  rm -f "$T/clicar"
done
touch "$T/fim"
[ "$FALHAS" -eq 0 ] && echo "TUDO CERTO" || echo "$FALHAS FALHA(S)"
exit "$FALHAS"
