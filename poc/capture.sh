#!/bin/sh
# KalangoOS · prova de conceito · roda tudo SEM janela (compositor em memória) e salva capturas em poc/capturas/.
# Serve para conferir o resultado sem depender da tela: ./capture.sh
. "$(dirname "$0")/env.sh"
OUT="$HERE/capturas"; mkdir -p "$OUT"; rm -f "$OUT"/*.png "$OUT"/mola.txt
INNER=$(mktemp --suffix=.sh)
cat > "$INNER" <<SH
#!/bin/sh
KALANGO_POC_TRACE=1 GSK_RENDERER=cairo $APP 2>"$OUT/mola.txt" &
sleep 1.5; grim "$OUT/1-fechada.png"
$BIN open controles; sleep 0.13; grim "$OUT/2-abrindo.png"; sleep 1.3; grim "$OUT/3-controles.png"
$BIN open avisos;    sleep 0.8;  grim "$OUT/4-avisos.png"
$BIN open agente;    sleep 0.8;  grim "$OUT/5-agente.png"
$BIN theme; $BIN open controles; sleep 0.8; grim "$OUT/6-tema-claro.png"
$BIN close; sleep 0.6
# o mesmo binário em modo de pré-visualização (janela comum, sem layer-shell)
$BIN quit; sleep 0.6
KALANGO_POC_PREVIEW=1 KALANGO_POC_WALLPAPER=$WALL GSK_RENDERER=cairo $APP &
sleep 1.5; $BIN open controles; sleep 1.2; grim "$OUT/7-previa-em-janela.png"; $BIN quit; sleep 0.3
swaymsg exit
SH
chmod +x "$INNER"
CONF=$(mktemp --suffix=.kalango-poc.conf)
cat > "$CONF" <<CFG
output * bg $WALL fill
output * resolution 1920x1080
default_border none
exec $INNER
CFG
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 timeout 60 sway --unsupported-gpu -c "$CONF" 2>"$HERE/.deps/sway-capture.log"
ls "$OUT"
