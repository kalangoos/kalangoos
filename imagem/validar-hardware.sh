#!/bin/bash
# KalangoOS · validação em hardware de verdade. Rode na SESSÃO AO VIVO (ou num sistema instalado) de um computador real:
#
#   imagem/validar-hardware.sh                 pergunta antes de suspender (padrão: não suspende)
#   imagem/validar-hardware.sh --suspender     faz o teste de suspender e voltar, sem perguntar
#   imagem/validar-hardware.sh --sem-suspender não testa o suspender e não pergunta
#   imagem/validar-hardware.sh --saida PASTA   onde gravar o relatório (padrão: ./kalango-validacao-DATA-HASH)
#   (baixe este arquivo do repositório: a imagem 0.1.0 ainda não o traz; na sessão ao vivo use o navegador ou curl)
#
# Escreve uma PASTA com um resumo (resumo.md, para colar numa issue) e os detalhes de cada área. Não grava em disco
# nenhum, não instala nada, não precisa de root: o que só o root lê (as mensagens do kernel) usa `sudo -n` quando a
# sessão ao vivo deixa, e senão o relatório diz que ficou sem. Nada pessoal sai: endereços MAC, IPs, números de série,
# UUIDs, nome de usuário, nome do computador e redes Wi-Fi são trocados por <marcadores> ANTES de gravar, e no fim o
# script procura o que sobrou e avisa. Leia a pasta antes de enviar (é texto puro).
#
# Cada passo termina em ok (funcionou), aviso (funcionou, mas vale olhar), falha ou não testado; o que não existe no
# computador (sem Bluetooth, por exemplo) é "ausente", não falha.
set -u
export LC_ALL=C   # (os rótulos do lscpu, systemd-analyze e nmcli são lidos por este script: sempre em inglês)
VERSION=1
SUSPEND=ask OUT=
while [ $# -gt 0 ]; do
  case $1 in
    --suspender) SUSPEND=yes ;;
    --sem-suspender) SUSPEND=no ;;
    --saida) OUT=${2:?pasta}; shift ;;
    -h|--help|--ajuda) sed -n 2,17p "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "argumento desconhecido: $1 (use --ajuda)"; exit 2 ;;
  esac
  shift
done

ME=$(id -un); MYHOST=$(hostname 2>/dev/null || echo host); MYUID=$(id -u)
# (os testes de imagem/testar-validar-hardware.sh trocam estes valores por nomes inventados)
[ "${KALANGO_VALIDAR_TESTE:-}" = 1 ] && { ME=${KALANGO_VALIDAR_USUARIO:-$ME}; MYHOST=${KALANGO_VALIDAR_COMPUTADOR:-$MYHOST}; }
STAMP=$(date +%Y%m%d-%H%M)
# (o sufixo vem do relógio e do kernel, só para duas execuções não se pisarem; não identifica o computador)
SUFFIX=$(printf '%s' "$STAMP$$$(cat /proc/sys/kernel/random/boot_id 2>/dev/null)" | sha256sum | cut -c1-6)
OUT=${OUT:-$PWD/kalango-validacao-$STAMP-$SUFFIX}
SUMMARY=$OUT/resumo.md ROWS=$OUT/.linhas

# ---------------------------------------------------------------------------------------------- privacidade
# Todas as pessoas com conta neste computador (uid >= 1000): o nome de cada uma é dado pessoal onde quer que apareça.
human_users() { { printf '%s\n' "$ME"; [ "${KALANGO_VALIDAR_TESTE:-}" = 1 ] || getent passwd 2> /dev/null | awk -F: '$3 >= 1000 && $3 < 65000 {print $1}'; } | sort -u | grep -vxE 'root|nobody' ; }
# Tudo o que vai para a pasta passa por aqui. Marcadores em vez de valores: o dado pessoal não existe na pasta.
redact() {
  local s u hexg='[0-9a-fA-F]{1,4}'
  s=$(sed -E \
    -e 's/([0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}/<mac>/g' \
    -e 's/\<enx[0-9a-f]{12}\>/enx<mac>/g' \
    -e 's/[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}/<uuid>/g' \
    -e 's/(^|[^0-9A-Za-z])[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}([^0-9A-Za-z]|$)/\1<uuid>\2/g' \
    -e 's/\\x2d/-/g' \
    -e 's/(^|[^0-9A-Za-z])[0-9a-f]{32}([^0-9A-Za-z]|$)/\1<id>\2/g' \
    -e 's/\<([0-9]{1,3}\.){3}[0-9]{1,3}(\/[0-9]{1,2})?\>/<ip>/g' \
    -e "s/(^|[^0-9A-Za-z:])(($hexg:){7}$hexg|($hexg:){1,6}:($hexg(:$hexg){0,5})?|:(:$hexg){1,7})(%[A-Za-z0-9_.-]+)?(\/[0-9]{1,3})?/\1<ip6>/g" \
    -e 's#(^|[^A-Za-z0-9_])([A-Za-z0-9._-]+\.(mount|automount|swap))\>#\1<montagem>#g' \
    -e 's#dev-disk-by-[A-Za-z]+-[^ ."]*#dev-disk-by-<id>#g' \
    -e 's#/dev/disk/by-[a-z]+/[^ ,;"]*#/dev/disk/by-<id>#g' \
    -e 's/\<(wwn-0x|eui\.|nvme-eui\.)[0-9a-fA-F]+/\1<id>/g' \
    -e 's/\<0x[0-9a-fA-F]{12,}\>/<id>/g' \
    -e 's/(^|[^A-Za-z])(([Ss]erial ?[Nn]umber|[Ss]erial|SerialNumber|iSerial|[Ss][Nn]|ID_SERIAL[A-Z_]*|WWN|wwn)[=: ]+)[^ ,;"]+/\1\2<removido>/g' \
    -e 's/((PART)?LABEL(_ENC)?=)[^ ,;"]+/\1<removido>/g' \
    -e 's#(^|[^A-Za-z])Device <mac> .*#\1Device <mac> <removido>#' \
    -e "s/(SSID|ssid|[Cc]onnection|[Pp]rofile|[Nn]etwork|[Aa]ssociat[a-z]*( with)?)([ =:]+)(\\\\?'[^']*\\\\?'|\\\\?\"[^\"]*\\\\?\")/\\1\\3<removido>/g" \
    -e "s/(SSID|ssid)[=: ]+[^ ,;'\"<]+/\\1 <removido>/g" \
    -e 's/(^|[^A-Za-z0-9])input: .* as \/devices/\1input: <dispositivo> as \/devices/' \
    -e 's#Controller (<mac>|[^ ]+) .*#Controller <mac> <removido>#' \
    -e "s#/home/[^/ :\"']+#/home/<usuario>#g" \
    -e 's#/(run/media|media|mnt)/[^/ :"'"'"']+#/\1/<usuario>#g' \
    -e 's#(AccountsService/users|/users)/[^/ :"'"'"']+#\1/<usuario>#g' \
    -e "s#/run/user/$MYUID\\b#/run/user/<uid>#g" \
    -e 's#user@[0-9]+\.service#user@<uid>.service#g')
  # (o nome de cada conta e o do computador: como palavra solta, depois de "/", "-" ou "@" nos caminhos e nas unidades;
  # "kalango-shell" e "os.kalango.Remote" ficam porque a conta ao vivo e o programa se chamam igual e são públicos)
  if [ -n "$MYHOST" ] && [ "$MYHOST" != kalangoos ] && [ "$MYHOST" != localhost ]; then
    s=$(printf '%s\n' "$s" | sed -E "s/(^|[^A-Za-z0-9_.-])$MYHOST(\.[A-Za-z0-9-]+)*([^A-Za-z0-9_.-]|\$)/\1<computador>\3/gI")
  fi
  for u in $(human_users); do
    [ "$u" = kalango ] && [ "${KALANGO_VALIDAR_TESTE:-}" != 1 ] && continue
    # (maiúsculas também, e "joao.", "joao_pc", "joao-pc": só letras e números ao lado fazem parte de outra palavra)
    s=$(printf '%s\n' "$s" | sed -E "s/(^|[^A-Za-z0-9])$u([^A-Za-z0-9]|\$)/\1<usuario>\2/gI; s/(^|[^A-Za-z0-9])$u([^A-Za-z0-9]|\$)/\1<usuario>\2/gI")
  done
  printf '%s\n' "$s"
}
# save NAME: lê o stdin, redige e grava OUT/NAME.txt
save() { redact > "$OUT/$1.txt"; }
# row ID STATUS TITLE [DETAIL]: uma linha do resumo (status: ok|aviso|falha|ausente|nao-testado)
row() { printf '%s|%s|%s|%s\n' "$1" "$2" "$3" "$(printf '%s' "${4:-}" | tr '\n|' '  ' | redact)" >> "$ROWS"; }
have() { command -v "$1" > /dev/null 2>&1; }
# sudo sem senha, nunca pergunta (a sessão ao vivo deixa; um sistema instalado talvez não)
rootcmd() { if [ "$MYUID" = 0 ]; then "$@"; elif sudo -n true 2> /dev/null; then sudo -n "$@"; else return 126; fi; }
say() { printf '  %s\n' "$*"; }
# O que `systemd-detect-virt` diz: "none" (e sai com 1) em hardware de verdade. Nunca devolve duas linhas.
detect_virt() { local v; v=$(systemd-detect-virt 2> /dev/null | head -1); printf '%s' "${v:-none}"; }
# Placas de rede de verdade: sem as pontes e os veth do docker, do libvirt e de contêineres.
net_devices() { nmcli -t -f DEVICE,TYPE,STATE device 2> /dev/null | grep -vE '^(lo|docker[0-9]*|veth[^:]*|br-[^:]*|virbr[^:]*|vnet[^:]*|tun[0-9]*|tap[0-9]*):'; }
# A linha do resumo sobre virtualização: "none" é hardware de verdade; qualquer outro valor é uma máquina virtual.
virt_row() {
  if [ "$1" != none ]; then row virt aviso "Rodou numa máquina virtual ($1), não em hardware" "um relatório de VM não conta para a matriz de hardware"
  else row virt ok "Rodou em hardware de verdade" ""; fi
}
# O que sobrou de dado pessoal numa pasta: uma linha por achado, nada se tudo está limpo. (Os marcadores <...> saem antes
# da busca, para uma linha com um marcador e um vazamento não passar.)
leftovers() {
  local d=$1 f u pat
  for f in "$d"/*.txt "$d"/*.md; do
    [ -f "$f" ] || continue
    sed -E 's/<[a-z0-9]+>//g; s/(^|[^0-9A-Za-z:])::1([^0-9A-Za-z:]|$)/\1\2/g' "$f" | grep -nE -e '([0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}' -e '\<([0-9]{1,3}\.){3}[0-9]{1,3}\>' \
      -e '[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}' -e '(^|[^0-9A-Za-z])[0-9A-F]{4}-[0-9A-F]{4}([^0-9A-Za-z]|$)' \
      -e '(^|[^0-9A-Za-z:])(([0-9a-fA-F]{1,4}:){1,7}:|::[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4})' \
      -e '\.(mount|automount|swap)\>' -e 'by-(id|uuid|label|partuuid)/[^<]' -e '\\x2d' -e "(SSID|[Cc]onnection|[Pp]rofile) *['\"][^'\"]" \
      -e '(PART)?LABEL(_ENC)?=[^< ]' -e '/home/[A-Za-z0-9._-]' -e '/(media|mnt)/[A-Za-z0-9._-]' | sed "s#^#$(basename "$f"):#"
    for u in $(human_users); do
      [ "$u" = kalango ] && [ "${KALANGO_VALIDAR_TESTE:-}" != 1 ] && continue
      sed -E 's/<[a-z0-9]+>//g' "$f" | grep -nwE -e "$u" | sed "s#^#$(basename "$f"): nome de usuário:#"
    done
    if [ -n "$MYHOST" ] && [ "$MYHOST" != kalangoos ] && [ "$MYHOST" != localhost ]; then
      sed -E 's/<[a-z0-9]+>//g' "$f" | grep -nF -e "$MYHOST" | sed "s#^#$(basename "$f"): nome do computador:#"
    fi
  done
}
# (para testar as funções acima sem rodar nada: KALANGO_VALIDAR_TESTE=1 . imagem/validar-hardware.sh)
[ "${KALANGO_VALIDAR_TESTE:-}" = 1 ] && return 0 2> /dev/null

mkdir -p "$OUT" || { echo "não consegui criar $OUT"; exit 1; }
: > "$ROWS"

echo "KalangoOS · validação de hardware (v$VERSION). Relatório em: $OUT"

# ---------------------------------------------------------------------------------------------- 1. o sistema
echo "1/10 sistema"
{
  echo "script: validar-hardware.sh v$VERSION"
  echo "data: $(date -u +%Y-%m-%dT%H:%MZ)"
  grep -E '^(PRETTY_NAME|VERSION|VERSION_ID|BUILD_ID)=' /etc/os-release 2> /dev/null
  echo "kernel: $(uname -r) $(uname -m)"
  echo "linha de comando do kernel: $(cat /proc/cmdline 2> /dev/null)"
  if [ -d /sys/firmware/efi ]; then echo "firmware: UEFI"; else echo "firmware: BIOS (legado)"; fi
  SB=$(od -An -t u1 /sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c 2> /dev/null | awk '{print $NF}')
  case ${SB:-} in 1) echo "Secure Boot: ligado" ;; 0) echo "Secure Boot: desligado" ;; *) echo "Secure Boot: não deu para ler" ;; esac
  if ls /sys/class/tpm/tpm0 > /dev/null 2>&1; then echo "TPM: presente"; else echo "TPM: ausente"; fi
  echo "virtualização: $(detect_virt)"
  grep -q 'boot=casper' /proc/cmdline 2> /dev/null && echo "modo: sessão ao vivo (pendrive)" || echo "modo: sistema instalado"
} | save sistema
VIRT=$(detect_virt)
LIVE=no; grep -q 'boot=casper' /proc/cmdline 2> /dev/null && LIVE=yes
virt_row "$VIRT"
if [ -d /sys/firmware/efi ]; then row uefi ok "Firmware UEFI" ""; else row uefi aviso "Firmware BIOS (legado)" "o instalador do KalangoOS pede UEFI"; fi

# ---------------------------------------------------------------------------------------------- 2. a máquina
echo "2/10 máquina (modelo, processador, memória)"
{
  for f in sys_vendor product_name product_family board_vendor board_name bios_vendor bios_version bios_date chassis_type; do
    [ -r /sys/class/dmi/id/$f ] && echo "$f: $(cat /sys/class/dmi/id/$f)"
  done
  lscpu 2> /dev/null | grep -E '^(Model name|Architecture|CPU\(s\)|Thread|Core|Socket|Flags)' | sed -E 's/^(Flags:).*/\1 (omitido)/'
  grep -E '^(MemTotal|SwapTotal)' /proc/meminfo
  echo "--- discos (sem número de série)"
  lsblk -d -o NAME,SIZE,TYPE,ROTA,TRAN,RM 2> /dev/null
} | save maquina
RAM_MB=$(awk '/MemTotal/ {print int($2/1024)}' /proc/meminfo)
CORES=$(nproc 2> /dev/null || echo 0)
MODEL="$(cat /sys/class/dmi/id/sys_vendor 2> /dev/null) $(cat /sys/class/dmi/id/product_name 2> /dev/null)"
CPU=$(lscpu 2> /dev/null | sed -n 's/^Model name: *//p' | head -1)
if [ "$RAM_MB" -ge 7500 ] && [ "$CORES" -ge 4 ]; then row req ok "Atende o mínimo (4 núcleos, 8 GB)" "$CORES núcleos, $RAM_MB MB"; else row req aviso "Abaixo do mínimo recomendado (4 núcleos, 8 GB)" "$CORES núcleos, $RAM_MB MB"; fi

# ---------------------------------------------------------------------------------------------- 3. PCI e USB
echo "3/10 dispositivos PCI e USB"
{
  if have lspci; then lspci -nnk 2> /dev/null; else echo "lspci não existe; /sys/bus/pci:"; for d in /sys/bus/pci/devices/*; do echo "$(basename "$d") $(cat "$d/vendor") $(cat "$d/device") driver=$(basename "$(readlink "$d/driver" 2> /dev/null)" 2> /dev/null)"; done; fi
  echo "--- USB (sem número de série)"
  if have lsusb; then lsusb 2> /dev/null; else
    for d in /sys/bus/usb/devices/*; do [ -r "$d/idVendor" ] && echo "$(cat "$d/idVendor"):$(cat "$d/idProduct") $(cat "$d/manufacturer" 2> /dev/null) $(cat "$d/product" 2> /dev/null)"; done
  fi
} | save pci-usb

# ---------------------------------------------------------------------------------------------- 4. vídeo
echo "4/10 placa de vídeo e driver"
GPUS=0 GPU_NOTES=
{
  for c in /sys/class/drm/card[0-9]*; do
    case $c in *-*) continue ;; esac
    d=$c/device
    [ -r "$d/vendor" ] || continue
    v=$(cat "$d/vendor"); id=$(cat "$d/device"); drv=$(basename "$(readlink "$d/driver" 2> /dev/null)" 2> /dev/null)
    case $v in 0x10de) vn=NVIDIA ;; 0x1002) vn=AMD ;; 0x8086) vn=Intel ;; 0x1af4|0x1b36) vn=virtio ;; 0x15ad) vn=VMware ;; 0x1234) vn=QEMU ;; *) vn=$v ;; esac
    echo "$(basename "$c"): fabricante=$vn id=$v:$id driver=${drv:-nenhum}"
  done
  echo "--- módulos de vídeo carregados"
  lsmod 2> /dev/null | grep -E '^(nvidia|nouveau|amdgpu|radeon|i915|xe|virtio_gpu|vmwgfx|bochs|simpledrm|drm)' || echo "(nenhum dos conhecidos)"
  if [ -r /proc/driver/nvidia/version ]; then echo "--- NVIDIA"; head -2 /proc/driver/nvidia/version; echo "nvidia_drm.modeset=$(cat /sys/module/nvidia_drm/parameters/modeset 2> /dev/null)"; fi
  have nvidia-smi && nvidia-smi --query-gpu=name,driver_version --format=csv,noheader 2> /dev/null
  echo "--- renderizador do compositor (niri)"
  journalctl --user -u niri -b --no-pager 2> /dev/null | grep -iE 'renderer|gles|vulkan|egl|gbm|drm device|primary node|explicit sync|vrr' | head -15
  have vulkaninfo && vulkaninfo --summary 2> /dev/null | grep -E 'deviceName|driverName|apiVersion' | head -8
  have glxinfo && glxinfo -B 2> /dev/null | grep -E 'renderer|OpenGL version' | head -3
} | save video
GPU_LINES=$(grep -E '^card[0-9]+:' "$OUT/video.txt")
GPUS=$(printf '%s\n' "$GPU_LINES" | grep -c .)
if [ "$GPUS" -eq 0 ]; then
  row gpu falha "Nenhuma placa de vídeo DRM encontrada" "sem /sys/class/drm/card*"
else
  VENDORS=$(printf '%s\n' "$GPU_LINES" | sed -E 's/.*fabricante=([^ ]+).*/\1/' | sort -u | tr '\n' ',' | sed 's/,$//')
  if printf '%s\n' "$GPU_LINES" | grep -q 'driver=nenhum'; then row gpu aviso "Vídeo: $VENDORS, mas há placa sem driver" "$(printf '%s' "$GPU_LINES" | tr '\n' ';')"
  elif printf '%s\n' "$GPU_LINES" | grep -qE 'fabricante=(virtio|VMware|QEMU)'; then row gpu aviso "Vídeo virtual ($VENDORS): não prova nada sobre GPU de verdade" "$(printf '%s' "$GPU_LINES" | tr '\n' ';')"
  else row gpu ok "Vídeo: $VENDORS" "$(printf '%s' "$GPU_LINES" | tr '\n' ';')"; fi
fi

# ---------------------------------------------------------------------------------------------- 5. kernel e firmware
echo "5/10 avisos do kernel e do firmware"
KLOG=
if KLOG=$(dmesg --level=err,warn 2> /dev/null) && [ -n "$KLOG" ]; then KSRC="dmesg"
elif KLOG=$(rootcmd dmesg --level=err,warn 2> /dev/null) && [ -n "$KLOG" ]; then KSRC="dmesg (sudo)"
elif KLOG=$(journalctl -k -b -p warning --no-pager 2> /dev/null) && [ -n "$KLOG" ]; then KSRC="journalctl -k"
else KSRC=""; fi
{
  if [ -n "$KSRC" ]; then
    echo "fonte: $KSRC; erros e avisos desta inicialização"
    printf '%s\n' "$KLOG" | head -150
  else
    echo "sem acesso às mensagens do kernel (precisa de root: rode com sudo, ou 'sudo sysctl kernel.dmesg_restrict=0')"
  fi
  echo "--- firmware que o kernel não achou"
  printf '%s\n' "$KLOG" | grep -iE 'firmware.*(failed|not found|direct-loading)|failed to load.*firmware|Direct firmware load' | head -20
  echo "--- kernel contaminado (taint): $(cat /proc/sys/kernel/tainted 2> /dev/null)"
} | save kernel
if [ -z "$KSRC" ]; then row kernel nao-testado "Mensagens do kernel não lidas" "falta permissão (dmesg)"
else
  NERR=$(printf '%s\n' "$KLOG" | grep -ciE 'error|fail|oops|BUG:|call trace')
  NFW=$(printf '%s\n' "$KLOG" | grep -ciE 'firmware.*(failed|not found)|failed to load.*firmware|Direct firmware load')
  if printf '%s\n' "$KLOG" | grep -qE 'Oops|BUG:|Call Trace|kernel panic'; then row kernel falha "O kernel registrou um oops/BUG" "veja kernel.txt"
  elif [ "$NFW" -gt 0 ]; then row kernel aviso "Firmware não encontrado ($NFW avisos); $NERR linhas com erro" "veja kernel.txt: em geral Wi-Fi, Bluetooth ou GPU sem o blob"
  else row kernel ok "Sem oops nem firmware faltando ($NERR linhas com 'error/fail')" ""; fi
fi

# ---------------------------------------------------------------------------------------------- 6. sessão e serviços
echo "6/10 sessão Wayland (niri), shell, kalangod e agente"
{
  echo "XDG_SESSION_TYPE=${XDG_SESSION_TYPE:-}  WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-}  XDG_CURRENT_DESKTOP=${XDG_CURRENT_DESKTOP:-}"
  ls "${XDG_RUNTIME_DIR:-/run/user/$MYUID}" 2> /dev/null | grep -E '^wayland-|^niri' | sed 's/^/socket: /'
  echo "--- processos da sessão"
  ps -u "$MYUID" -o comm= 2> /dev/null | grep -E 'niri|kalango|pipewire|wireplumber' | sort | uniq -c
  echo "--- serviços do usuário"
  systemctl --user list-units --no-legend --plain --type=service 2> /dev/null | grep -E 'niri|kalango|pipewire|wireplumber|greetd' | awk '{print $1, $3, $4}'
  echo "--- serviços do sistema"
  systemctl list-units --no-legend --plain --type=service 2> /dev/null | grep -E 'kalango|greetd|NetworkManager|bluetooth|systemd-logind|seatd|ssh' | awk '{print $1, $3, $4}'
  echo "--- unidades que falharam (sistema e usuário)"
  systemctl --failed --no-legend --plain 2> /dev/null
  systemctl --user --failed --no-legend --plain 2> /dev/null
  echo "--- agente (kalango-agente) e kalangod"
  ls /usr/libexec/kalango/ 2> /dev/null | grep -E 'agente|kalangod' | sed 's/^/instalado: /'
  [ -S /run/kalango/mcp.sock ] && echo "kalangod: socket /run/kalango/mcp.sock presente" || echo "kalangod: sem socket em /run/kalango/mcp.sock"
} | save sessao
if pgrep -u "$MYUID" -f 'niri.bin|/niri( |$)' > /dev/null 2>&1; then row niri ok "A sessão Wayland (niri) subiu" ""; else row niri falha "O niri não está rodando nesta sessão" "veja sessao.txt e video.txt"; fi
if pgrep -u "$MYUID" -f 'kalango-shell' > /dev/null 2>&1; then row shell ok "O shell do KalangoOS está de pé" ""; else row shell falha "O kalango-shell não está rodando" "veja sessao.txt"; fi
if systemctl is-active --quiet kalangod 2> /dev/null; then row kalangod ok "kalangod ativo" ""; else row kalangod falha "kalangod não está ativo" "systemctl status kalangod"; fi
if [ -x /usr/libexec/kalango/kalango-agente ] || have kalango-agente; then row agente ok "O motor dos agentes está instalado" "o teste de modelo real não faz parte deste script"; else row agente aviso "kalango-agente não encontrado" ""; fi
FAILED=$(systemctl --failed --no-legend --plain 2> /dev/null; systemctl --user --failed --no-legend --plain 2> /dev/null)
if [ -z "$FAILED" ]; then row units ok "Nenhum serviço falhou" ""; else row units falha "Serviços que falharam" "$FAILED"; fi

# ---------------------------------------------------------------------------------------------- 7. áudio, rede, Bluetooth
echo "7/10 áudio, rede e Bluetooth"
{
  echo "--- placas de som"; cat /proc/asound/cards 2> /dev/null || echo "(nenhuma)"
  # (os nomes dos aparelhos, como o "Fones da Maria", são do dono: ficam só os números, o tipo e o volume)
  echo "--- PipeWire (sem os nomes dos aparelhos)"; have wpctl && wpctl status 2> /dev/null | sed -n '1,30p' | sed -E 's/^([ │├└─*]*[0-9]+\.) .*(\[vol:[^]]*\])?$/\1 <dispositivo> \2/'
  echo "--- rede (tipo e estado; sem nomes de redes)"
  have nmcli && net_devices
  echo "--- rfkill"
  for r in /sys/class/rfkill/rfkill*; do [ -r "$r/type" ] && echo "$(cat "$r/type"): soft=$(cat "$r/soft") hard=$(cat "$r/hard")"; done
  echo "--- Bluetooth"; have bluetoothctl && timeout 5 bluetoothctl list 2> /dev/null | sed -E 's/Controller [^ ]+/Controller <mac>/'
  ls /sys/class/bluetooth 2> /dev/null | sed 's/^/adaptador: /'
} | save audio-rede-bluetooth
if grep -qE '^ *[0-9]+ \[' /proc/asound/cards 2> /dev/null; then
  SINKS=$(wpctl status 2> /dev/null | sed -n '/Sinks:/,/Sources:/p' | grep -cE '^ *│? *[*]? *[0-9]+\.')
  if [ "$SINKS" -gt 0 ]; then row audio ok "Placa de som e saída do PipeWire" "$SINKS saída(s)"; else row audio aviso "Placa de som existe, mas o PipeWire não mostra saída" "wpctl status"; fi
else row audio falha "Nenhuma placa de som" "/proc/asound/cards vazio"; fi
NET=$(net_devices | cut -d: -f2-)
WIFI=$(printf '%s\n' "$NET" | grep -c '^wifi:')
ETH=$(printf '%s\n' "$NET" | grep -c '^ethernet:')
if ! have nmcli; then row rede nao-testado "Rede: o nmcli não existe neste sistema" "não deu para listar as placas de rede"
elif printf '%s\n' "$NET" | grep -q ':connected'; then row rede ok "Rede conectada ($ETH com fio, $WIFI Wi-Fi)" ""; elif [ "$((WIFI+ETH))" -gt 0 ]; then row rede aviso "Há placa de rede, nenhuma conexão ativa" "$ETH com fio, $WIFI Wi-Fi; conecte e rode de novo, ou diga na issue"; else row rede falha "Nenhuma placa de rede" ""; fi
if [ "$WIFI" -gt 0 ]; then row wifi nao-testado "Há adaptador Wi-Fi (conectar é um teste manual)" "veja docs/VALIDACAO-HARDWARE.md"; else row wifi ausente "Sem adaptador Wi-Fi" ""; fi
if ls /sys/class/bluetooth/hci* > /dev/null 2>&1; then row bluetooth ok "Adaptador Bluetooth presente" "parear um aparelho é teste manual"; else row bluetooth ausente "Sem Bluetooth" ""; fi

# ---------------------------------------------------------------------------------------------- 8. telas
echo "8/10 telas e modos"
{
  echo "--- conectores (kernel)"
  for s in /sys/class/drm/card*-*; do [ -r "$s/status" ] && echo "$(basename "$s"): $(cat "$s/status") $(head -3 "$s/modes" 2> /dev/null | tr '\n' ' ')"; done
  echo "--- niri msg outputs (o nome do monitor, que traz o número de série, é omitido)"
  if have niri && [ -n "${WAYLAND_DISPLAY:-}" ]; then
    NIRI_SOCKET=${NIRI_SOCKET:-$(ls "${XDG_RUNTIME_DIR:-/run/user/$MYUID}"/niri.*.sock 2> /dev/null | head -1)} timeout 8 niri msg outputs 2> /dev/null | sed -E 's/^(Output) ".*" (\(.*\))/\1 "<removido>" \2/'
  else echo "(sem sessão Wayland para perguntar)"; fi
} | save telas
CONN=$(grep -c ': connected' "$OUT/telas.txt")
if [ "$CONN" -gt 0 ]; then row telas ok "$CONN tela(s) conectada(s) com modos" "$(grep -E 'Current mode|current' "$OUT/telas.txt" | head -3 | tr '\n' ';')"; else row telas falha "Nenhuma tela conectada vista pelo kernel" ""; fi

# ---------------------------------------------------------------------------------------------- 9. instalador (sem gravar)
echo "9/10 instalador, só a simulação (nada é gravado em disco)"
INST=/usr/libexec/kalango/kalango-instalador; [ -x "$INST" ] || INST=$(command -v kalango-instalador || true)
{
  if [ -n "$INST" ]; then
    PLAN=$(mktemp)
    # (o disco é /dev/null: sem KALANGO_INSTALADOR_REAL e sem root o motor só escreve o que FARIA; nada toca disco)
    cat > "$PLAN" <<'PLAN'
{"disk": "/dev/null", "disk_title": "Disco de simulação", "mode": "Erase", "size": 0, "encrypt": false,
 "locale": "pt_BR.UTF-8", "keyboard": "br", "name": "Teste", "user": "teste", "hostname": "teste-kalango",
 "autologin": false, "fingerprint_later": false, "avatar": null,
 "models": {"fonte": "padrao", "escolhidos": [], "rede_local_confirmada": false}}
PLAN
    # (duas linhas no stdin: a senha e o token do servidor de modelos, ambos vazios de verdade)
    printf 'simulacao-sem-senha\n\n' | env -u KALANGO_INSTALADOR_REAL timeout 120 "$INST" --instalar "$PLAN" 2>&1
    echo "rc=${PIPESTATUS[1]}"
    rm -f "$PLAN"
  else echo "kalango-instalador não encontrado"; fi
  echo "--- discos que o instalador veria (só tipo e tamanho)"
  lsblk -d -o NAME,SIZE,TYPE,RM,TRAN 2> /dev/null
} > "$OUT/instalador.raw" 2>&1
redact < "$OUT/instalador.raw" > "$OUT/instalador.txt"; rm -f "$OUT/instalador.raw"
if [ -z "$INST" ]; then row instalador aviso "Instalador não encontrado neste sistema" ""
elif grep -q '"modo":"simulado"' "$OUT/instalador.txt" && grep -q '^rc=0' "$OUT/instalador.txt" && ! grep -q '"estado":"erro"' "$OUT/instalador.txt"; then row instalador ok "O motor do instalador simulou a instalação inteira, sem gravar" "veja instalador.txt"
else row instalador falha "A simulação do instalador não terminou" "veja instalador.txt"; fi
NDISK=$(lsblk -d -n -o TYPE 2> /dev/null | grep -c '^disk$')
if [ "$NDISK" -gt 0 ]; then row discos ok "$NDISK disco(s) visível(is) ao instalador" ""; else row discos aviso "Nenhum disco visível" ""; fi

# ---------------------------------------------------------------------------------------------- 10. tempos e memória (+ suspender)
echo "10/10 tempo de boot, memória em repouso e suspender"
{
  echo "--- tempo de boot"; systemd-analyze time 2> /dev/null | head -2
  for u in niri kalango-shell; do
    us=$(systemctl --user show "$u.service" -p ActiveEnterTimestampMonotonic --value 2> /dev/null)
    [ -n "${us:-}" ] && [ "$us" != 0 ] && echo "$u.service ficou ativo $((us/1000000)) s depois do começo do kernel"
  done
  echo "uptime agora: $(cut -d' ' -f1 /proc/uptime) s"
  echo "--- memória (MiB usados = total - disponível), 3 amostras com 5 s"
  for i in 1 2 3; do awk '/MemTotal/ {t=$2} /MemAvailable/ {a=$2} END {printf "usado: %d MiB de %d MiB\n", (t-a)/1024, t/1024}' /proc/meminfo; [ $i -lt 3 ] && sleep 5; done
  echo "--- processos da sessão por RSS (MiB)"
  ps -u "$MYUID" -o rss=,comm= --sort=-rss 2> /dev/null | head -8 | awk '{printf "%7.0f  %s\n", $1/1024, $2}'
  echo "soma RSS da sessão: $(ps -u "$MYUID" -o rss= 2> /dev/null | awk '{s+=$1} END {printf "%d MiB", s/1024}')"
} | save desempenho
SHELLS=$(sed -n 's/^kalango-shell.service ficou ativo \([0-9]*\) s.*/\1/p' "$OUT/desempenho.txt")
USED=$(sed -n 's/^usado: \([0-9]*\) MiB.*/\1/p' "$OUT/desempenho.txt" | sort -n | head -1)
SESSRSS=$(sed -n 's/^soma RSS da sessão: \([0-9]*\) MiB/\1/p' "$OUT/desempenho.txt")
[ -n "${SHELLS:-}" ] && row boot ok "O shell ficou de pé ${SHELLS} s depois do começo do kernel" "$(systemd-analyze time 2> /dev/null | head -1)" || row boot nao-testado "Tempo até o shell não medido" "faltou systemctl --user show"
if [ -n "${SESSRSS:-}" ]; then
  # (a meta de docs/REQUISITOS.md é 900 MB da sessão em repouso; ao vivo e com o instalador aberto, passa um pouco)
  if [ "$SESSRSS" -le 900 ]; then row memoria ok "Sessão em repouso: ${SESSRSS} MiB de RSS (meta: até 900)" "sistema todo: ${USED:-?} MiB usados"; else row memoria aviso "Sessão em repouso: ${SESSRSS} MiB de RSS (meta: até 900)" "sistema todo: ${USED:-?} MiB; ao vivo e com o instalador aberto passa um pouco"; fi
fi

# suspender: só com a pessoa confirmando (pode travar o computador; salve o que importa antes)
DO_SUSPEND=$SUSPEND
if [ "$SUSPEND" = ask ]; then
  if [ -t 0 ] && [ -t 1 ]; then
    printf '\nTestar suspender e voltar? A tela apaga por alguns segundos; se o computador não voltar, aperte o botão de ligar. [s/N] '
    read -r ans; case ${ans:-n} in s|S|y|Y) DO_SUSPEND=yes ;; *) DO_SUSPEND=no ;; esac
  else DO_SUSPEND=no; fi
fi
if [ "$DO_SUSPEND" = yes ]; then
  if [ ! -r /sys/power/state ] || ! grep -q mem /sys/power/state; then
    row suspender ausente "O kernel não oferece suspender (mem) neste computador" "$(cat /sys/power/state 2> /dev/null)"
  else
    echo "  suspendendo por 12 s (o relógio de alarme acorda o computador)..."
    MEMSLEEP=$(cat /sys/power/mem_sleep 2> /dev/null)
    T0=$(date +%s)
    if have rtcwake && rootcmd rtcwake -m no -s 12 > /dev/null 2>&1; then :; fi
    systemctl suspend 2> /dev/null || rootcmd systemctl suspend 2> /dev/null
    sleep 3; T1=$(date +%s)
    { echo "mem_sleep: $MEMSLEEP"; echo "pediu suspender às $(date -d @$T0 +%H:%M:%S), voltou às $(date -d @$T1 +%H:%M:%S) (relógio local)"
      echo "--- mensagens do kernel sobre PM depois da volta"; (dmesg 2> /dev/null || rootcmd dmesg 2> /dev/null) | grep -iE 'PM:|suspend|resume|ACPI: PM|failed|error' | tail -25; } | save suspender
    sleep 4
    if pgrep -u "$MYUID" -f niri > /dev/null 2>&1 && pgrep -u "$MYUID" -f kalango-shell > /dev/null 2>&1; then
      if [ "$((T1 - T0))" -ge 8 ]; then row suspender ok "Suspendeu e voltou ($((T1 - T0)) s) com a sessão de pé" "mem_sleep: $MEMSLEEP"; else row suspender aviso "Voltou depressa demais ($((T1 - T0)) s): talvez não tenha suspendido de verdade" "mem_sleep: $MEMSLEEP"; fi
    else row suspender falha "Voltou do suspender sem o niri ou o shell" "veja suspender.txt"; fi
  fi
else
  row suspender nao-testado "Suspender e voltar: a pessoa não confirmou" "rode com --suspender"
fi

# ---------------------------------------------------------------------------------------------- resumo
fmt() { case $1 in ok) echo "ok";; aviso) echo "AVISO";; falha) echo "FALHA";; ausente) echo "ausente";; *) echo "não testado";; esac; }
{
  echo "## KalangoOS · validação de hardware"
  echo
  echo "- **Máquina:** $(printf '%s' "$MODEL" | tr -s ' ') · $(printf '%s' "$CPU" | tr -s ' ') · $CORES núcleos · $RAM_MB MB"
  echo "- **KalangoOS:** $(sed -n 's/^PRETTY_NAME=//p' /etc/os-release 2> /dev/null | tr -d '"') · kernel $(uname -r) · $([ "$LIVE" = yes ] && echo 'sessão ao vivo' || echo 'instalado') · $([ -d /sys/firmware/efi ] && echo UEFI || echo BIOS) · virtualização: $VIRT"
  echo "- **Vídeo:** ${VENDORS:-nenhum}  ($(printf '%s' "$GPU_LINES" | sed -E 's/^card[0-9]+: //; s/ id=[^ ]+//' | tr '\n' ';'))"
  echo "- **Script:** v$VERSION, $(date -u +%Y-%m-%d)"
  echo
  echo "| Passo | Resultado | O que viu |"
  echo "|---|---|---|"
  while IFS='|' read -r id st title detail; do
    printf '| %s | %s | %s%s |\n' "$id" "$(fmt "$st")" "$title" "${detail:+ — $detail}"
  done < "$ROWS"
  echo
  echo "Detalhes (texto puro, já sem dados pessoais) na pasta \`$(basename "$OUT")\`: sistema, maquina, pci-usb, video, kernel, sessao, audio-rede-bluetooth, telas, instalador, desempenho."
  echo
  echo "**Teste manual (marque você):** entrar na conta [ ]  Wi-Fi [ ]  som [ ]  monitor externo [ ]  Remote entre dois computadores [ ]  suspender [ ]"
} > "$SUMMARY"

# ---------------------------------------------------------------------------------------------- confere o que sobrou
LEFT=$(leftovers "$OUT")
if [ -n "$LEFT" ]; then
  echo "AVISO: ficou algo que parece dado pessoal; leia a pasta antes de enviar:"
  printf '%s\n' "$LEFT" | cut -c1-140 | head -10 | sed 's/^/  /'
  printf '\n> Revise a pasta antes de enviar: o filtro de dados pessoais achou algo suspeito.\n' >> "$SUMMARY"
fi
rm -f "$ROWS"
echo
cat "$SUMMARY"
echo
echo "Pronto. Copie a pasta para um pendrive (a sessão ao vivo some ao desligar):"
echo "  $OUT"
echo "e cole o resumo numa issue: https://github.com/kalangoos/kalangoos/issues/new?template=hardware.yml"
