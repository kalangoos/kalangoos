#!/usr/bin/env python3
"""KalangoOS · conversa com uma VM da ISO pelo console serial (imagem/testar-iso.sh): entra, roda comandos e confere.

    testar-iso.py <serial.sock> ao-vivo           a sessão ao vivo: niri, shell e o instalador de pé, nada falhando
    testar-iso.py <serial.sock> instalar <plano>  roda o motor do instalador de verdade no disco da VM e desliga
    testar-iso.py <serial.sock> instalado <usuário> <senha>   o sistema instalado: a sessão da conta criada de pé
    testar-iso.py <serial.sock> iommu <usuário> <senha> <url>  gpu.iommu_ligar/desligar de um kalangod novo (baixado de
                                                  <url>) e, depois de cada reinício, a linha do kernel (imagem/testar-iommu.sh)

Sai com 0 quando tudo confere; senão diz o que não conferiu (e o que o sistema registrou) e sai com 1.
"""
import base64
import re
import socket
import sys
import time

ESC = re.compile(r"\x1b\][^\x07\x1b]*(\x07|\x1b\\)|\x1b\[[0-9;?]*[a-zA-Z]|\x1bP[^\x1b]*\x1b\\")


class Console:
    def __init__(self, path):
        self.s = socket.socket(socket.AF_UNIX)
        self.s.connect(path)
        self.s.settimeout(1)
        self.n = 0

    def read(self, seconds):
        end, out = time.time() + seconds, b""
        while time.time() < end:
            try:
                out += self.s.recv(1 << 16)
            except socket.timeout:
                pass
            except (ConnectionResetError, BrokenPipeError):
                # (a VM desligou e fechou o console: o que veio até ali vale)
                break
        return ESC.sub("", out.decode(errors="replace"))

    def until(self, pattern, seconds):
        end, out = time.time() + seconds, ""
        while time.time() < end:
            out += self.read(2)
            if re.search(pattern, out):
                return out
        raise TimeoutError(f"não apareceu /{pattern}/ em {seconds} s; o fim do console:\n{out[-1500:]}")

    def send(self, text):
        self.s.send(text.encode() + b"\n")

    def login(self, user, password, seconds):
        self.send("")
        # (o console pode já estar com a conta aberta, de um passo anterior na mesma VM)
        if not re.search(r"login: *$", self.until(r"(login: *$|\$ *$)", seconds)):
            return
        self.send(user)
        # (o prompt da senha no idioma do sistema: "Password:" sem os pacotes de idioma, "Senha:" com eles)
        out = self.until(r"((Password|Senha): *$|\$ *$)", 30)
        if re.search(r"(Password|Senha): *$", out):
            self.send(password)
            self.until(r"\$ *$", 30)
        # (sem eco, um prompt que não atrapalha a leitura das respostas)
        self.send("stty -echo; export PS1='$ ' PROMPT_COMMAND=")
        self.read(2)

    def run(self, cmd, seconds=60, stdin=None):
        """A saída de `cmd` e o código de saída dele."""
        self.n += 1
        tag = f"__KALANGO_{self.n}__"
        self.send(f"{cmd}; echo {tag} $?")
        if stdin is not None:
            time.sleep(1)
            self.send(stdin)
        out = self.until(rf"{tag} \d+", seconds)
        body, code = out.rsplit(tag, 1)
        return body.strip(), int(code.split()[0])


def check(ok, what, details=""):
    print(("ok    " if ok else "FALHOU ") + what)
    if not ok and details:
        print("      " + details.replace("\n", "\n      "))
    return ok


def session_up(c, user, seconds=240):
    """Espera o niri, o shell (e, ao vivo, o instalador) da conta de pé."""
    wanted = ["niri.bin --session", "kalango-shell.bin"]
    end = time.time() + seconds
    while time.time() < end:
        out, _ = c.run(f"ps -u {user} -o args=")
        if all(w in out for w in wanted):
            return out
        time.sleep(5)
    return out


def main():
    sock, mode = sys.argv[1], sys.argv[2]
    c = Console(sock)
    good = True
    if mode == "ao-vivo":
        c.login("kalango", "", 900)
        procs = session_up(c, "kalango")
        good &= check("niri.bin --session" in procs and "kalango-shell.bin" in procs, "a sessão ao vivo sobe (niri e shell)", procs)
        good &= check("kalango-instalador" in procs, "o instalador abre sozinho", procs)
        host, _ = c.run("hostname")
        good &= check(host.strip() == "kalangoos", "o computador ao vivo se chama kalangoos", host)
        time.sleep(60)
        procs, _ = c.run("ps -u kalango -o args=")
        good &= check("niri.bin --session" in procs and "kalango-shell.bin" in procs, "a sessão continua de pé 1 min depois", procs)
        failed, _ = c.run("systemctl --failed --no-legend --plain")
        good &= check(failed.strip() == "", "nenhum serviço falhou", failed)
        # (a placa com fio da VM ligada pelo NetworkManager: com a regra do Ubuntu sem netplan, ela ficava sem gerência)
        net, _ = c.run("nmcli -t -f TYPE,STATE device | grep ^ethernet")
        good &= check("ethernet:connected" in net, "a rede com fio conecta sozinha", net)
    elif mode == "instalar":
        plan = sys.argv[3]
        password = sys.argv[4]
        c.login("kalango", "", 900)
        # (numa linha só: o comando ganha o "; echo" do fim, que um heredoc não aceitaria)
        b64 = base64.b64encode(open(plan, "rb").read()).decode()
        c.run(f"echo {b64} | base64 -d > /tmp/plano.json")
        out, code = c.run("sudo env KALANGO_INSTALADOR_REAL=1 /usr/libexec/kalango/kalango-instalador --instalar /tmp/plano.json", 1800, stdin=password)
        steps = [l for l in out.splitlines() if l.startswith("{")]
        print("      " + "\n      ".join(l[:160] for l in steps if '"estado":"feito"' in l.replace(" ", "") or "erro" in l or "fim" in l or "modo" in l))
        good &= check(code == 0 and '"fim":true' in out.replace(" ", ""), "o motor instalou no disco da VM", out[-1500:])
        c.send("sudo poweroff")
        c.read(5)
    elif mode == "iommu":
        user, password, url = sys.argv[3], sys.argv[4], sys.argv[5]

        def start_kalangod():
            c.login(user, password, 900)
            c.run(f"python3 -c \"import urllib.request as u; [u.urlretrieve('{url}/'+n, '/tmp/'+n) for n in ('kalangod', 'chamar-kalangod.py')]\" && chmod +x /tmp/kalangod", 120)
            c.run("sudo -S -p '' true", 30, stdin=password)
            c.run("sudo rm -rf /tmp/kd; sudo -b sh -c '/tmp/kalangod servir --dev /tmp/kd --sem-terminal > /tmp/kd.log 2>&1'; sleep 3; sudo cat /tmp/kd.log")

        def call(tool):
            return c.run(f"sudo env KALANGOD=/tmp/kalangod KALANGOD_SOCKET=/tmp/kd/mcp.sock python3 /tmp/chamar-kalangod.py {tool} 2>&1", 300)

        def reboot():
            c.send("sudo reboot")
            c.read(20)

        start_kalangod()
        before, _ = c.run("cat /proc/cmdline")
        print("      antes: " + before)
        good &= check("iommu=pt" not in before, "a linha do kernel começa sem o IOMMU", before)
        out, code = call("gpu.iommu_ligar")
        good &= check(code == 0, "gpu.iommu_ligar com aprovação", out)
        cfg, _ = c.run("cat /etc/default/grub.d/95-kalangoos-iommu.cfg; ls /etc/default/grub.d/; sudo grep -c 'iommu=pt' /boot/grub/grub.cfg")
        good &= check("$GRUB_CMDLINE_LINUX_DEFAULT" in cfg and "90-kalangoos.cfg" in cfg, "o arquivo 95 ao lado do 90 do instalador, e o grub.cfg refeito", cfg)
        reboot()
        start_kalangod()
        on, _ = c.run("cat /proc/cmdline; ls /sys/kernel/iommu_groups | wc -l")
        print("      depois de ligar: " + on.replace("\n", " · "))
        vendor = "intel_iommu=on" if "intel" in c.run("grep -m1 vendor_id /proc/cpuinfo")[0].lower() else "amd_iommu=on"
        good &= check(vendor in on and "iommu=pt" in on and "quiet splash" in on, "depois do reinício a linha do kernel tem o IOMMU (e o que o instalador pôs)", on)
        out, code = call("gpu.iommu_desligar")
        good &= check(code == 0, "gpu.iommu_desligar com aprovação", out)
        gone, _ = c.run("ls /etc/default/grub.d/; sudo grep -c 'iommu=pt' /boot/grub/grub.cfg")
        good &= check("95-kalangoos-iommu.cfg" not in gone, "o arquivo 95 saiu", gone)
        reboot()
        c.login(user, password, 900)
        off, _ = c.run("cat /proc/cmdline")
        print("      depois de desligar: " + off)
        good &= check("iommu=pt" not in off and vendor not in off and "quiet splash" in off, "depois do reinício a linha do kernel volta sem o IOMMU", off)
        c.run("sudo -S -p '' true", 30, stdin=password)
        c.send("sudo poweroff")
        c.read(5)
    elif mode == "instalado":
        user, password = sys.argv[3], sys.argv[4]
        c.login(user, password, 900)
        procs = session_up(c, user)
        good &= check("niri.bin --session" in procs and "kalango-shell.bin" in procs, f"o sistema instalado abre a sessão de {user}", procs)
        good &= check("kalango-instalador" not in procs, "o instalador não abre no sistema instalado", procs)
        # (pelo estado do dpkg, que não depende do idioma do sistema)
        casper, _ = c.run("dpkg-query -W -f='${Status}' casper 2>/dev/null")
        good &= check("installed" not in casper.split(), "o casper (o boot ao vivo) saiu do sistema instalado", casper)
        # (o teclado escolhido no instalador, pelo /etc/default/keyboard que o systemd-localed lê e o niri pergunta)
        kb, _ = c.run(f"XDG_RUNTIME_DIR=/run/user/$(id -u) NIRI_SOCKET=$(ls /run/user/$(id -u)/niri.*.sock | head -1) niri msg keyboard-layouts")
        good &= check("Portuguese (Brazil)" in kb, "o teclado do instalador (br) vale na sessão", kb)
        # (o gerenciador de pacotes e as fontes do Ubuntu: sem eles nada se instala nem se atualiza)
        apt, _ = c.run("command -v apt-get; grep -h '^Suites:' /etc/apt/sources.list.d/*.sources | tr '\\n' ' '; ls /usr/share/keyrings/ubuntu-archive-keyring.gpg")
        good &= check("/usr/bin/apt-get" in apt and "-security" in apt and "ubuntu-archive-keyring" in apt, "o apt e as fontes do Ubuntu (com as de segurança) estão lá", apt)
        # (os apps nativos de repositório próprio: o atalho no menu e nenhuma biblioteca faltando)
        apps, _ = c.run("ls /usr/share/applications/os.kalango.Remote.desktop /usr/share/applications/os.kalango.Audio.desktop; "
                        "LD_LIBRARY_PATH=/usr/lib/kalango/remote ldd /usr/lib/kalango/remote/kalango-remote /usr/lib/kalango/remote/kalango-remote-core /usr/bin/kalango-audio | grep -c 'not found'")
        good &= check("os.kalango.Remote.desktop" in apps and "os.kalango.Audio.desktop" in apps and apps.strip().endswith("0"),
                      "o Kalango Remote e o Kalango Audio estão no sistema, sem biblioteca faltando", apps)
        # (a / do root e 755: senão o systemd-tmpfiles recusa caminhos e qualquer conta mexe na raiz)
        top, _ = c.run("stat -c '%U:%G %a' /")
        good &= check(top.strip() == "root:root 755", "a raiz / é do root, com 755", top)
        root, _ = c.run("findmnt -no FSTYPE,SOURCE /")
        good &= check(root.startswith("ext4"), "a raiz é o disco instalado (ext4)", root)
        # (o carregador reserva, que qualquer UEFI acha sem a entrada na memória do firmware)
        # (a tela de entrada, que a entrada automática pula: o programa que o niri dela abre existe — com o caminho errado
        # ela fechava na hora e o greetd desistia, deixando a tela preta para quem tem senha)
        greeter, _ = c.run("p=$(sed -n 's/^spawn-sh-at-startup \"\\([^; ]*\\).*/\\1/p' /usr/share/kalango/entrada.kdl); echo $p; test -x \"$p\" && echo existe")
        good &= check(greeter.strip().endswith("existe"), "a tela de entrada abre o programa dela", greeter)
        # (a partição EFI é só do root: o ls com sudo)
        fb, _ = c.run("sudo -S -p '' ls /boot/efi/EFI/BOOT/BOOTX64.EFI /boot/efi/EFI/KalangoOS/", 30, stdin=password)
        good &= check("BOOTX64.EFI" in fb and "grubx64.efi" in fb, "o disco liga sozinho (EFI/BOOT/BOOTX64.EFI e a entrada KalangoOS)", fb)
        efi, _ = c.run("findmnt -no FSTYPE /boot/efi")
        good &= check(efi.strip() == "vfat", "a partição EFI está montada", efi)
        failed, _ = c.run("systemctl --failed --no-legend --plain")
        good &= check(failed.strip() == "", "nenhum serviço falhou", failed)
        # (desempenho: os números de cada ISO, para comparar uma com a outra — o boot, a memória e a CPU com a sessão
        # parada, e quem mais pesa)
        time.sleep(30)
        perf, _ = c.run("systemd-analyze | head -1; free -m | sed -n 2p | awk '{print \"RAM usada: \" $3 \" MB de \" $2}'; "
                        "cat /proc/loadavg | awk '{print \"carga (1 min): \" $1}'; "
                        "ps -eo rss,pcpu,comm --sort=-rss | head -8 | awk 'NR>1 {printf \"  %6d MB %5s%% %s\\n\", $1/1024, $2, $3}'")
        print("---- desempenho\n" + perf)
        if failed.strip():
            units = " ".join(l.split()[0] for l in failed.splitlines() if l.strip())
            why, _ = c.run(f"sudo -S -p '' journalctl -b --no-pager -o cat -u {units.replace(' ', ' -u ')} | tail -30", 30, stdin=sys.argv[4])
            print("---- o registro dos serviços que falharam\n" + why)
    if not good and mode != "instalar":
        log, _ = c.run("sudo -S -p '' journalctl -b --no-pager -p warning | tail -60", 30, stdin=sys.argv[4] if mode == "instalado" else "")
        print("---- o registro do sistema (avisos e erros)\n" + log)
    if mode == "instalado":
        c.send("sudo -S poweroff")
        time.sleep(1)
        c.send(sys.argv[4])
        c.read(3)
    sys.exit(0 if good else 1)


if __name__ == "__main__":
    main()
