import json, os, subprocess, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
os.environ.setdefault("KALANGOD", "")
import importlib.util
spec = importlib.util.spec_from_file_location("t", os.path.join(os.path.dirname(os.path.abspath(__file__)), "testes.py"))
src = open(spec.origin).read().replace("\nmain()\n", "\n")  # reaproveita Cliente/check/aprova sem rodar os testes da VM
t = {}
exec(compile(src, "testes.py", "exec"), t)
SOCK = os.environ["SOCK"]; t["SOCK"] = SOCK
Cliente, check, v, falhas = t["Cliente"], t["check"], t["v"], t["falhas"]
run = lambda *a: subprocess.run(list(a), capture_output=True, text=True)

def agente():
    os.setgroups([]); os.setgid(1000); os.setuid(1000); os.environ["HOME"] = "/home/home"
    c = Cliente(SOCK)
    r = v(c.call("tarefas.executar", {"programa": "sh", "argumentos": ["-c", "id -u; cat /proc/self/cgroup"]}))
    check("systemd: tarefa como uid 1000", r.get("saida", "").startswith("1000\n"), r)
    check("systemd: escopo em /kalango.slice/kalango-agentes.slice", "/kalango.slice/kalango-agentes.slice/kalango-tarefa-" in r.get("saida", "") and r.get("modo") == "systemd", r)
    r = v(c.call("tarefas.executar", {"programa": "python3", "argumentos": ["-c", "x=bytearray(800*1024*1024)"], "memoria_mb": 128}))
    check("systemd: limite de memória", r.get("estourou_memoria") is True and r.get("sinal") == 9, r)
    r = v(c.call("tarefas.executar", {"programa": "sleep", "argumentos": ["20"], "tempo_s": 1}))
    check("systemd: limite de tempo", r.get("estourou_tempo") is True, r)
    sys.exit(1 if falhas else 0)

pid = os.fork()
if pid == 0:
    agente()
_, st = os.waitpid(pid, 0)
if os.waitstatus_to_exitcode(st): falhas.append("agente")

c = Cliente(SOCK)
ks = v(c.call("kernel.seguranca"))
check("host sem bpf no lsm=: guarda desligada e dizendo por quê", ks.get("guarda_ebpf_ativa") is False and "eBPF LSM inativo" in ks.get("guarda_ebpf", ""), ks)

# rede: conexão dummy criada só para o teste
run("nmcli", "connection", "delete", "kalango-teste")
a = run("nmcli", "connection", "add", "type", "dummy", "ifname", "kdummy0", "con-name", "kalango-teste", "autoconnect", "no", "ipv4.method", "manual", "ipv4.addresses", "10.254.254.1/32", "ipv6.method", "disabled")
check("cria conexão dummy de teste", a.returncode == 0, a.stderr)
cs = v(c.call("rede.conexoes"))["conexoes"]
check("rede.conexoes lista a conexão", any(x["nome"] == "kalango-teste" for x in cs), cs)
r = c.call("rede.conexao", {"nome": "kalango-teste", "acao": "ativar"}, aprovar=True)
ativa = lambda: any(x["nome"] == "kalango-teste" and x["estado"] == "activated" for x in v(c.call("rede.conexoes"))["conexoes"])
check("rede.conexao ativa", not r["isError"] and ativa(), r)
u = c.call("linha_do_tempo.desfazer", {"id": v(r).get("registro", 0)}, aprovar=True)
time.sleep(1)
check("desfazer desativa", not u["isError"] and not ativa(), u)
run("nmcli", "connection", "delete", "kalango-teste")
w = c.call("rede.wifi")
check("rede.wifi responde", not w["isError"], w)

# discos: imagem loop com ext4
run("truncate", "-s", "64M", "/tmp/kalango-disco.img"); run("mkfs.ext4", "-q", "-F", "/tmp/kalango-disco.img")
dev = run("losetup", "-f", "--show", "/tmp/kalango-disco.img").stdout.strip()
montado = lambda: run("findmnt", "-n", "-S", dev).returncode == 0
r = c.call("discos.montar", {"dispositivo": dev}, aprovar=True)
check(f"discos.montar {dev} (udisks)", not r["isError"] and montado(), r)
u = c.call("linha_do_tempo.desfazer", {"id": v(r).get("registro", 0)}, aprovar=True)
check("desfazer desmonta", not u["isError"] and not montado(), u)
run("losetup", "-d", dev); os.remove("/tmp/kalango-disco.img")
disks = [b for b in v(c.call("discos.dispositivos"))["blockdevices"] if b["type"] == "disk" and not b["name"].startswith(("loop", "zram"))]
if disks:
    h = v(c.call("discos.saude", {"dispositivo": disks[0]["path"]}))
    check(f"discos.saude {disks[0]['path']}", h.get("saudavel") in (True, False), h)
    print(f"         {disks[0]['path']}: {h.get('modelo')} saudável={h.get('saudavel')} temp={h.get('temperatura_c')}")

# pontos no /etc real: cria e só planeja (não restaura o host)
p = v(c.call("pontos.criar", {"descricao": "validação no host"}, aprovar=True)).get("ponto", {})
check("ponto real do /etc do host", p.get("tipo") == "arquivos" and p.get("pacotes", 0) > 1000, p)
pl = c.call("pontos.restaurar", {"id": p.get("id", ""), "so_plano": True}, aprovar=True)
check("plano de restauração sem mudanças de pacote", not pl["isError"] and v(pl)["plano"]["pacotes"] == {"instalar": [], "remover": []}, v(pl).get("plano"))
print("KALANGO-HOST: " + ("TUDO OK" if not falhas else f"{len(falhas)} FALHAS: {falhas}"))
sys.exit(1 if falhas else 0)
