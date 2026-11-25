#!/bin/bash
# KalangoOS · testes das funções de imagem/validar-hardware.sh com comandos de mentira (sem rede, sem root, sem tocar
# em nada): a virtualização (none/kvm/qemu), a redação de dados pessoais (com nomes inventados) e a conferência final.
#   imagem/testar-validar-hardware.sh      sai com 0 quando tudo confere
set -u
cd "$(dirname "$0")/.."
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
mkdir -p "$T/bin" "$T/out"
FALHAS=0
ok() { if eval "$2"; then echo "ok     $1"; else echo "FALHOU $1"; FALHAS=$((FALHAS + 1)); fi; }

# --- os comandos de mentira: systemd-detect-virt como em hardware de verdade (diz "none" e SAI COM 1), em KVM e em QEMU
stub_virt() { printf '#!/bin/sh\n%s\n' "$1" > "$T/bin/systemd-detect-virt"; chmod +x "$T/bin/systemd-detect-virt"; }
export PATH="$T/bin:$PATH"
export KALANGO_VALIDAR_TESTE=1 KALANGO_VALIDAR_USUARIO=joao KALANGO_VALIDAR_COMPUTADOR=casa-do-joao OUT="$T/out"
# shellcheck disable=SC1091
. imagem/validar-hardware.sh
OUT="$T/out"; SUMMARY=$OUT/resumo.md ROWS=$OUT/.linhas; : > "$ROWS"

stub_virt 'echo none; exit 1'
ok "hardware de verdade: detect_virt devolve uma linha só, 'none'" '[ "$(detect_virt)" = none ]'
virt_row "$(detect_virt)"
ok "hardware de verdade: a linha do resumo é 'ok' e não fala de VM" 'grep -q "^virt|ok|Rodou em hardware" "$ROWS" && ! grep -q "máquina virtual" "$ROWS"'
: > "$ROWS"
for v in kvm qemu; do
  stub_virt "echo $v"
  ok "$v: detect_virt devolve '$v'" '[ "$(detect_virt)" = '$v' ]'
  virt_row "$(detect_virt)"
  ok "$v: a linha do resumo é um aviso de VM, numa linha" 'grep -q "^virt|aviso|Rodou numa máquina virtual ('$v')" "$ROWS" && [ "$(grep -c "^virt|" "$ROWS")" = 1 ]'
  : > "$ROWS"
done
stub_virt 'exit 1'
ok "systemd-detect-virt sem saída vira 'none'" '[ "$(detect_virt)" = none ]'

# --- as placas de rede: sem as pontes e os veth do docker
printf '#!/bin/sh\nprintf "%%s\\n" "lo:loopback:connected" "enp3s0:ethernet:connected" "docker0:bridge:connected" "veth1a2b3c:ethernet:connected" "br-0f1e2d:bridge:connected" "wlp2s0:wifi:disconnected"\n' > "$T/bin/nmcli"; chmod +x "$T/bin/nmcli"
ok "as placas de rede não contam docker0, veth nem br-" '[ "$(net_devices | cut -d: -f1 | tr "\n" " ")" = "enp3s0 wlp2s0 " ]'

# --- a redação, com nomes inventados (usuário joao, computador casa-do-joao); nada disto é de uma pessoa de verdade
cat > "$T/amostra.txt" <<'AMOSTRA'
inet6 2001:db8::1/64 scope global
inet6 fe80::1ff:fe23:4567:890a%wlp3s0 scope link
addr 2001:db8:85a3:0:0:8a2e:370:7334 loopback ::1
gateway 192.0.2.7 e 198.51.100.0/24
mac aa:bb:cc:dd:ee:ff e enxaabbccddeeff
/media/joao/PENDRIVE e /run/media/joao/DISCO
media-joao-PENDRIVE.mount carregado
run-media-joao-DISCO.automount
-.mount e home-joao-nuvem.mount e dev-sda1.swap
var/lib/AccountsService/users/joao
/home/joao/Documentos/x e user@1000.service
dev-disk-by\x2duuid-5E3A\x2d12F4.device e UUID=5E3A-12F4 e 123e4567-e89b-12d3-a456-426614174000
host casa-do-joao.lan e casa-do-joao e CASA-DO-JOAO.local
NetworkManager: connection 'CasaDaMaria5G' activated em wlp2s0
wpa_supplicant: Associated with SSID "Cafe Central"
SSID=RedeDoVizinho
/dev/disk/by-id/ata-Samsung_SSD_860_EVO_S3Z9NB0K123456X-part1 wwn-0x5002538e40a1b2c3
nvme-eui.0025385b71b1c2d3
Controller AA:BB:CC:DD:EE:FF Notebook da Ana [default]
input: Teclado da Maria as /devices/virtual/misc/uhid/0005
usb 1-1: SerialNumber: XYZ123456
joao logou
JOAO e Joao entraram; joao.silva, joao_pc e joao-pc
linux root=LABEL=MeuDiscoDoJoao e PARTLABEL=casa-da-maria
sn: ABC987654 e Sn=ZZZ111222
Device AA:BB:CC:DD:EE:01 Fone da Maria
/home/maria/x /media/pedro/USB /var/lib/AccountsService/users/ana
machine-id 3b05073d481b4cafbf0727846d2c1296 em /run/user/1000/bus
nvme-eui.0025385b71b1c2d3 e wwn-0x5002538e40a1b2c3 e 0x5002538e40a1b2c3
AMOSTRA
redact < "$T/amostra.txt" > "$T/redigido.txt"
for segredo in 2001:db8 fe80::1ff 192.0.2 198.51.100 aa:bb:cc enxaabbcc /media/joao /run/media/joao media-joao run-media-joao home-joao AccountsService/users/joao /home/joao 5E3A 123e4567 casa-do-joao CASA-DO-JOAO CasaDaMaria5G "Cafe Central" RedeDoVizinho S3Z9NB0K 5002538e AA:BB:CC "Notebook da Ana" "Teclado da Maria" XYZ123456 MeuDiscoDoJoao casa-da-maria ABC987654 ZZZ111222 "Fone da Maria" joao_pc joao.silva /home/maria /media/pedro users/ana 3b05073d481b4cafbf 0025385b71b1 5002538e40a1 /run/user/1000 joao; do
  ok "redigido: '$segredo' não aparece" '! grep -qiF -- "$segredo" "$T/redigido.txt"'
done
ok "os marcadores aparecem (<ip6>, <ip>, <mac>, <usuario>, <montagem>, <computador>, <uuid>)" 'for m in "<ip6>" "<ip>" "<mac>" "<usuario>" "<montagem>" "<computador>" "<uuid>"; do grep -qF -- "$m" "$T/redigido.txt" || exit 1; done'
ok "o que não é pessoal fica (/usr, -.mount não é um segredo)" 'echo "/usr/libexec/kalango/kalango-shell.bin e kalango-aprovacao" | redact | grep -q "kalango-shell.bin e kalango-aprovacao"'

# --- a conferência do fim
mkdir -p "$T/limpo" "$T/sujo"
cp "$T/redigido.txt" "$T/limpo/a.txt"
ok "conferência: a pasta redigida passa limpa" '[ -z "$(leftovers "$T/limpo")" ]'
i=0
while IFS= read -r l; do
  i=$((i + 1)); printf '%s\n' "<ip> $l" > "$T/sujo/a.txt"
  case $l in *"é um segredo"*) continue ;; esac
done < /dev/null
for amostra in "inet6 2001:db8::1/64" "fe80::1ff:fe23:4567:890a%wlp3s0" "192.0.2.7" "aa:bb:cc:dd:ee:ff" "/media/joao/PENDRIVE" "media-joao-PENDRIVE.mount" "UUID=5E3A-12F4" 'dev-disk-by\x2duuid-5E3A' "casa-do-joao.lan" "connection 'CasaDaMaria5G'" "/dev/disk/by-id/ata-Samsung_S3Z9" "/home/joao/x" "joao logou"; do
  printf '<ip> %s <mac>\n' "$amostra" > "$T/sujo/a.txt"
  ok "conferência: acha '$amostra' mesmo numa linha que já tem marcadores" '[ -n "$(leftovers "$T/sujo")" ]'
done
echo
[ "$FALHAS" -eq 0 ] && echo "tudo confere" || { echo "$FALHAS falha(s)"; exit 1; }
