"""Testes do kalangod como root dentro da VM (eBPF LSM ligado, cgroups, usuários, firewall, pontos, btrfs)."""
import json, os, socket, subprocess, sys, threading, time

K = os.environ["KALANGOD"]
SOCK = "/run/kalango/mcp.sock"
falhas = []

def check(nome, ok, detalhe=""):
    print(("  ok   " if ok else "  FALHOU ") + nome + ("" if ok else f"\n         {str(detalhe)[:1500]}"), flush=True)
    if not ok:
        falhas.append(nome)

class Cliente:
    def __init__(self, sock=SOCK):
        self.s = socket.socket(socket.AF_UNIX); self.s.connect(sock); self.f = self.s.makefile("rw"); self.n = 0; self.sock = sock
    def rpc(self, m, p=None):
        self.n += 1
        self.f.write(json.dumps({"jsonrpc": "2.0", "id": self.n, "method": m, "params": p or {}}) + "\n"); self.f.flush()
        return json.loads(self.f.readline())
    def call(self, nome, args=None, aprovar=False):
        t = None
        if aprovar:
            t = threading.Thread(target=aprova, args=(self.sock,)); t.start()
        r = self.rpc("tools/call", {"name": nome, "arguments": args or {}})["result"]
        if t: t.join()
        return r

def aprova(sock):
    for _ in range(400):
        c = Cliente(sock); p = c.rpc("kalango/pendentes")["result"]
        if p:
            subprocess.run([K, "aprovar", str(p[0]["id"])], env={**os.environ, "KALANGOD_SOCKET": sock}, capture_output=True); return
        time.sleep(0.05)

def como_usuario(uid):
    """Os próximos sockets são abertos por um processo filho com esse uid (o agente)."""
    pass

def v(r): return r.get("structuredContent") or {}

def linha_do_tempo(c):
    # pela ferramenta: a pasta de estado é 0700 e cada usuário só vê os próprios registros
    return v(c.call("linha_do_tempo.listar", {"limite": 500}))["registros"]

def parte_agente():
    """Roda como uid 1000: o agente de uma pessoa."""
    c = Cliente()
    ks = v(c.call("kernel.seguranca"))
    check("eBPF LSM ativo no kernel", ks.get("bpf_lsm_ativo") is True, ks)
    check("guarda eBPF carregada pelo kalangod", ks.get("guarda_ebpf_ativa") is True, ks.get("guarda_ebpf"))
    # tarefa como a pessoa, em cgroup próprio dentro de kalango-agentes.slice
    r = v(c.call("tarefas.executar", {"programa": "sh", "argumentos": ["-c", "id -u; cat /proc/self/cgroup"]}))
    check("tarefa roda com o uid de quem pediu", r.get("saida", "").startswith("1000\n"), r)
    check("tarefa roda no cgroup dos agentes (cgroupfs)", "/kalango.slice/kalango-agentes.slice/kalango-tarefa-" in r.get("saida", "") and r.get("modo") == "cgroupfs", r)
    # um processo dentro do cgroup dos agentes é reconhecido como agente pelo kalangod (regra por cgroup)
    r = v(c.call("tarefas.executar", {"programa": K, "argumentos": ["quem"]}))
    check("tarefa é identificada pelo cgroup como agente nativo", '"regra": "agentes-nativos"' in r.get("saida", ""), r)
    # regra do kernel: ~/.ssh fica fora do alcance mesmo estando dentro da casa (onde o Landlock deixaria)
    r = v(c.call("tarefas.executar", {"programa": "ls", "argumentos": ["/home/home/.ssh"]}))
    check("kernel bloqueia agente em ~/.ssh", r.get("codigo") != 0 and "Operation not permitted" in r.get("erros", ""), r)
    r = v(c.call("tarefas.executar", {"programa": "cat", "argumentos": ["/home/home/.local/share/kalango/cofre.json"]}))
    check("kernel bloqueia agente no cofre de segredos", r.get("codigo") != 0 and "Operation not permitted" in r.get("erros", ""), r)
    r = v(c.call("tarefas.executar", {"programa": "sh", "argumentos": ["-c", "echo x > /home/home/kalango-vm-teste.txt && cat /home/home/kalango-vm-teste.txt"]}))
    check("agente escreve na própria casa", r.get("saida") == "x\n", r)
    # limites
    r = v(c.call("tarefas.executar", {"programa": "python3", "argumentos": ["-c", "x=bytearray(800*1024*1024)"], "memoria_mb": 128}))
    check("tarefa morre ao passar do limite de memória", r.get("estourou_memoria") is True and r.get("sinal") == 9, r)
    r = v(c.call("tarefas.executar", {"programa": "sleep", "argumentos": ["20"], "tempo_s": 1}))
    check("tarefa encerrada ao passar do tempo", r.get("estourou_tempo") is True and r.get("tempo_ms", 99999) < 4000, r)
    r = v(c.call("tarefas.executar", {"programa": "python3", "argumentos": ["-c", "import os,time\nn=0\nfor i in range(40):\n  try:\n    if os.fork()==0:\n      time.sleep(2); os._exit(0)\n    n+=1\n  except OSError: pass\nprint(n)"], "processos": 8}))
    check("limite de processos", r.get("saida", "99").strip().isdigit() and int(r["saida"]) < 8, r)
    left = os.listdir("/sys/fs/cgroup/kalango.slice/kalango-agentes.slice")
    check("cgroups das tarefas removidos ao fim", not [d for d in left if d.startswith("kalango-tarefa-")], left)
    # operações de arquivo (auxiliar com uid 1000 + Landlock + seccomp, lançado por um kalangod root)
    r = c.call("arquivos.escrever", {"caminho": "~/kalango-vm-arq.txt", "conteudo": "ok"})
    st = os.stat("/home/home/kalango-vm-arq.txt") if os.path.exists("/home/home/kalango-vm-arq.txt") else None
    check("arquivos.escrever cria com o dono certo (auxiliar com seccomp)", not r["isError"] and st and st.st_uid == 1000, r)
    # nível total: root, sem Landlock, mas ainda agente → /etc/shadow bloqueado pelo kernel
    r = v(c.call("sistema.executar", {"programa": "sh", "argumentos": ["-c", "id -u; head -c 4 /etc/shadow"]}, aprovar=True))
    check("nível total roda como root", r.get("saida", "").startswith("0\n"), r)
    check("kernel bloqueia até root-agente em /etc/shadow", "Operation not permitted" in r.get("erros", ""), r)
    time.sleep(2)
    tl = linha_do_tempo(c)
    bloq = [e for e in tl if e["tool"] == "auditoria.bloqueio"]
    check("bloqueio de ~/.ssh na linha do tempo da pessoa", any(e["args"]["nome"] == ".ssh" for e in bloq), bloq[-5:])
    check("a pessoa não lê a pasta de estado direto", not os.access("/var/lib/kalango/linha-do-tempo.jsonl", os.R_OK))
    check("auditoria de execução", any(e["tool"] == "auditoria.execucao" and "python3" in e["args"]["nome"] for e in tl), [e for e in tl if e["tool"] == "auditoria.execucao"][-3:])
    check("auditoria de escrita", any(e["tool"] == "auditoria.escrita" and e["args"]["nome"] == "kalango-vm-teste.txt" for e in tl))
    return c

def parte_sistema(c):
    bloq = [e for e in linha_do_tempo(c) if e["tool"] == "auditoria.bloqueio"]
    check("bloqueio de /etc/shadow (agente root) na linha do tempo", any(e["args"]["nome"] == "shadow" for e in bloq), bloq[-5:])
    # usuários
    r = c.call("usuarios.criar", {"nome": "kalangoteste", "nome_completo": "Teste VM"}, aprovar=True)
    existe = subprocess.run(["id", "kalangoteste"], capture_output=True).returncode == 0
    check("usuarios.criar cria a conta e a casa", not r["isError"] and existe and os.path.isdir("/home/kalangoteste"), r)
    check("ação de sistema cria ponto de restauração antes", "arquivos" in str(v(r).get("ponto_de_restauracao")), v(r))
    g = c.call("usuarios.grupo", {"nome": "kalangoteste", "grupo": "video", "acao": "adicionar"}, aprovar=True)
    em = "video" in subprocess.run(["id", "-nG", "kalangoteste"], capture_output=True, text=True).stdout
    check("usuarios.grupo adiciona", em, g)
    c.call("linha_do_tempo.desfazer", {"id": v(g)["registro"]}, aprovar=True)
    em = "video" in subprocess.run(["id", "-nG", "kalangoteste"], capture_output=True, text=True).stdout
    check("desfazer tira do grupo", not em)
    u = c.call("linha_do_tempo.desfazer", {"id": v(r)["registro"]}, aprovar=True)
    existe = subprocess.run(["id", "kalangoteste"], capture_output=True).returncode == 0
    check("desfazer remove a conta criada", not existe and not os.path.exists("/home/kalangoteste"), u)
    r = c.call("usuarios.remover", {"nome": "root"}, aprovar=True)
    check("não remove conta do sistema", r["isError"], r)
    # firewall
    r = c.call("firewall.ligar", {"ligado": True}, aprovar=True)
    check("firewall.ligar", not r["isError"], r)
    r = c.call("firewall.regra", {"acao": "permitir", "porta": "8080/tcp"}, aprovar=True)
    est = v(c.call("firewall.estado"))
    check("firewall.regra permite 8080/tcp", est.get("ligado") is True and any("8080/tcp" in x for x in est.get("regras", [])), est)
    c.call("linha_do_tempo.desfazer", {"id": v(r).get("registro", 0)}, aprovar=True)
    est = v(c.call("firewall.estado"))
    check("desfazer remove a regra", not any("8080/tcp" in x for x in est.get("regras", [])), est)
    # atualizações: índice real pela rede da VM
    r = c.call("atualizacoes.verificar", {}, aprovar=True)
    check("atualizacoes.verificar (apt-get update)", not r["isError"], r)
    lst = v(c.call("atualizacoes.listar")).get("atualizacoes", [])
    check("atualizacoes.listar", isinstance(lst, list), lst)
    print(f"         {len(lst)} atualizações disponíveis na VM")
    if lst:
        alvo = lst[0]["pacote"]
        antes = subprocess.run(["dpkg-query", "-W", "-f", "${Version}", alvo], capture_output=True, text=True).stdout
        r = c.call("atualizacoes.aplicar", {"pacotes": [alvo]}, aprovar=True)
        depois = subprocess.run(["dpkg-query", "-W", "-f", "${Version}", alvo], capture_output=True, text=True).stdout
        check(f"atualizacoes.aplicar {alvo} {antes} → {depois}", not r["isError"] and depois != antes and v(r).get("desfazivel") is True, r)
        u = c.call("linha_do_tempo.desfazer", {"id": v(r).get("registro", 0)}, aprovar=True)
        volta = subprocess.run(["dpkg-query", "-W", "-f", "${Version}", alvo], capture_output=True, text=True).stdout
        check(f"desfazer volta {alvo} para {antes}", not u["isError"] and volta == antes, (u, volta))
    else:
        check("há atualização para testar atualizacoes.aplicar", False, "nenhuma disponível")

def parte_pontos(c):
    p = v(c.call("pontos.criar", {"descricao": "teste VM"}, aprovar=True))["ponto"]
    check("ponto real do /etc + pacotes", p["tipo"] == "arquivos" and p["pacotes"] > 500 and p["bytes"] > 100000, p)
    # muda o sistema por fora do kalangod: conta nova, arquivo novo, arquivo alterado, pacote removido
    subprocess.run(["useradd", "-M", "intruso"], check=True)
    open("/etc/kalango-intruso.conf", "w").write("x")
    host_antes = open("/etc/hostname").read()
    open("/etc/hostname", "w").write("mudado\n")
    subprocess.run(["dpkg", "-r", "ncdu"], capture_output=True, check=True)
    pl = v(c.call("pontos.restaurar", {"id": p["id"], "so_plano": True}, aprovar=True))["plano"]
    check("plano: apagar arquivo novo, trocar alterados", "kalango-intruso.conf" in pl["etc"]["apagar"] and "hostname" in pl["etc"]["trocar"] and "passwd" in pl["etc"]["trocar"], pl["etc"])
    check("plano: reinstalar ncdu", any(x.startswith("ncdu:amd64=") for x in pl["pacotes"]["instalar"]), pl["pacotes"])
    r = c.call("pontos.restaurar", {"id": p["id"]}, aprovar=True)
    intruso = subprocess.run(["id", "intruso"], capture_output=True).returncode == 0
    ncdu = subprocess.run(["dpkg-query", "-W", "-f", "${db:Status-Abbrev}", "ncdu"], capture_output=True, text=True).stdout.startswith("ii")
    check("restaurar: /etc volta (conta, arquivo novo, hostname)", not r["isError"] and not intruso and not os.path.exists("/etc/kalango-intruso.conf") and open("/etc/hostname").read() == host_antes, r)
    check("restaurar: pacote removido volta (apt pela rede)", ncdu, v(r).get("saida_pacotes", "")[-800:])
    u = c.call("linha_do_tempo.desfazer", {"id": v(r)["registro"]}, aprovar=True)
    intruso = subprocess.run(["id", "intruso"], capture_output=True).returncode == 0
    check("desfazer a restauração volta ao estado de antes", not u["isError"] and intruso and os.path.exists("/etc/kalango-intruso.conf"), u)

def parte_btrfs():
    img, mnt = "/tmp/btrfs.img", "/mnt/raiz-btrfs"
    subprocess.run(["truncate", "-s", "512M", img], check=True)
    subprocess.run(["mkfs.btrfs", "-q", img], check=True)
    os.makedirs(mnt, exist_ok=True)
    m = subprocess.run(["mount", "-o", "loop", img, mnt], capture_output=True, text=True)
    check("monta btrfs de teste", m.returncode == 0, m.stderr)
    if m.returncode: return
    os.makedirs(f"{mnt}/etc/rede"); open(f"{mnt}/etc/hostname", "w").write("kalango\n"); open(f"{mnt}/etc/rede/cfg", "w").write("a\n")
    sock = "/run/kalango/btrfs.sock"
    cfg = open("/etc/kalango/kalangod.toml").read().replace('socket = "/run/kalango/mcp.sock"', f'socket = "{sock}"').replace('state_dir = "/var/lib/kalango"', 'state_dir = "/var/lib/kalango-btrfs"')
    cfg = cfg.replace("pontos_max = 20", f'pontos_max = 20\nrestore_root = "{mnt}"')
    open("/etc/kalango/btrfs.toml", "w").write(cfg)
    d = subprocess.Popen([K, "servir", "--config", "/etc/kalango/btrfs.toml"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(100):
        if os.path.exists(sock): break
        time.sleep(0.05)
    c = Cliente(sock)
    p = v(c.call("pontos.criar", {"descricao": "btrfs"}, aprovar=True)).get("ponto", {})
    subs = subprocess.run(["btrfs", "subvolume", "list", mnt], capture_output=True, text=True).stdout
    check("ponto btrfs é snapshot somente leitura", p.get("tipo") == "btrfs" and ".kalango-pontos/" in subs, (p, subs))
    ro = subprocess.run(["btrfs", "property", "get", p.get("snapshot", "/x"), "ro"], capture_output=True, text=True).stdout.strip()
    check("snapshot ro=true", ro == "ro=true", ro)
    open(f"{mnt}/etc/hostname", "w").write("mudou\n"); os.remove(f"{mnt}/etc/rede/cfg"); open(f"{mnt}/etc/novo", "w").write("n")
    r = c.call("pontos.restaurar", {"id": p["id"]}, aprovar=True)
    ok = open(f"{mnt}/etc/hostname").read() == "kalango\n" and os.path.exists(f"{mnt}/etc/rede/cfg") and not os.path.exists(f"{mnt}/etc/novo")
    check("restaurar do snapshot btrfs", not r["isError"] and ok, r)
    d.terminate()

def main():
    # parte do agente roda como uid 1000 num processo filho
    pid = os.fork()
    if pid == 0:
        os.setgroups([]); os.setgid(1000); os.setuid(1000)
        os.environ["HOME"] = "/home/home"
        c = parte_agente()
        sys.exit(1 if falhas else 0)
    _, st = os.waitpid(pid, 0)
    if os.waitstatus_to_exitcode(st) != 0:
        falhas.append("parte do agente")
    c = Cliente()
    parte_sistema(c)
    parte_pontos(c)
    parte_btrfs()
    print("KALANGO-VM: " + ("TUDO OK" if not falhas else f"{len(falhas)} FALHAS: {falhas}"), flush=True)
    sys.exit(1 if falhas else 0)

main()
