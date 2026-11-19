#!/bin/sh
# KalangoOS · prova de conceito · mede o custo da barra: memória, CPU parada, CPU animando, tempo de início.
# Roda num compositor sem janela, com desenho por software (o pior caso para a CPU). Uso: ./measure.sh
. "$(dirname "$0")/env.sh"
export KALANGO_POC_APP_ID=os.kalango.SidebarPoc.Medida
OUT="$HERE/capturas/medidas.txt"; mkdir -p "$HERE/capturas"
INNER=$(mktemp --suffix=.sh)
cat > "$INNER" <<SH
#!/bin/sh
cpu() { awk '{print \$14 + \$15}' /proc/\$1/stat; }          # tiques de CPU (100 por segundo)
t0=\$(date +%s%N)
GSK_RENDERER=cairo $APP 2>/dev/null &
gdbus wait --session --timeout 10 \$KALANGO_POC_APP_ID
t1=\$(date +%s%N)
sleep 1.5
PID=\$(pgrep -n -f "release/kalango-sidebar-poc\$")
{
echo "início até responder: \$(( (t1 - t0) / 1000000 )) ms"
a=\$(cpu \$PID); sleep 5; b=\$(cpu \$PID)
echo "CPU fechada e parada, 5 s: \$(( (b - a) * 10 )) ms"
$BIN open controles; sleep 2
a=\$(cpu \$PID); sleep 5; b=\$(cpu \$PID)
echo "CPU aberta e parada, 5 s: \$(( (b - a) * 10 )) ms"
a=\$(cpu \$PID); for i in 1 2 3 4 5; do $BIN close; sleep 0.5; $BIN open controles; sleep 0.5; done; b=\$(cpu \$PID)
echo "CPU em 10 animações (5 s): \$(( (b - a) * 10 )) ms"
awk '/^Rss:/{r=\$2} /^Pss:/{p=\$2} END{printf "memória: RSS %.1f MB · PSS (parte só dela) %.1f MB\\n", r/1024, p/1024}' /proc/\$PID/smaps_rollup
echo "threads: \$(ls /proc/\$PID/task | wc -l)"
echo "binário: \$(( \$(stat -c %s $BIN) / 1024 )) KB"
} > "$OUT"
$BIN quit; sleep 0.3
swaymsg exit
SH
chmod +x "$INNER"
CONF=$(mktemp --suffix=.kalango-poc.conf)
printf 'output * bg %s fill\noutput * resolution 1920x1080\ndefault_border none\nexec %s\n' "$WALL" "$INNER" > "$CONF"
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 timeout 60 sway --unsupported-gpu -c "$CONF" 2>/dev/null
cat "$OUT"
