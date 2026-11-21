"""KalangoOS · the agents' allowlist as it ships: kalangod as root (imagem/testar-lista-branca.sh runs this in the live
ISO's VM, as the live user, with this repository's kalangod in /tmp/kalangod). Starts kalangod with a test config and
checks, through it: who a command runs as, its own /dev/shm, writing inside and outside the allowed folders, Python's
process pools, the cgroup kill, local sockets, the per-grant caches and the person-only methods. Prints ok/FALHOU per
check and exits 1 when one failed."""
import json, os, socket, subprocess, sys, threading, time
KD, SOCK, H = "/tmp/kalangod", "/tmp/kd/mcp.sock", os.path.expanduser("~")
PY = os.path.realpath(sys.executable)
ok_all = True

def check(cond, what, detail=""):
    global ok_all
    ok_all &= bool(cond)
    print(("ok    " if cond else "FALHOU ") + what + (f"\n        {detail}" if detail else ""), flush=True)

def rpc(m, p):
    s = socket.socket(socket.AF_UNIX); s.connect(SOCK); f = s.makefile("rw")
    f.write(json.dumps({"jsonrpc": "2.0", "id": 1, "method": m, "params": p}) + "\n"); f.flush()
    return json.loads(f.readline())

def call(tool, args):
    return rpc("tools/call", {"name": tool, "arguments": args}).get("result", {})

def sh(script, pasta):
    r = call("tarefas.executar", {"programa": "sh", "argumentos": ["-c", script], "pasta": pasta})
    return r.get("structuredContent") or {"erro": r}

def approve_next(always=False):
    def run():
        for _ in range(300):
            p = rpc("kalango/pendentes", {}).get("result") or []
            if p:
                subprocess.run([KD, "aprovar", str(p[0]["id"])] + (["--sempre"] if always else []), env={**os.environ, "KALANGOD_SOCKET": SOCK}, capture_output=True)
                return
            time.sleep(0.1)
    t = threading.Thread(target=run); t.start(); return t

# ---- the test config and kalangod as root
for d in ["Documentos", "Projetos/site", ".config"]:
    os.makedirs(f"{H}/{d}", exist_ok=True)
open(f"{H}/.bashrc", "a").close()
open("/tmp/k.toml", "w").write(f'''socket = "{SOCK}"
state_dir = "/tmp/kd/estado"
protected_paths = ["~/.ssh", "~/.config/kalango"]
approval_requires_tty = false
approval_timeout_s = 60
total_grant_s = 900
dry_run = false
restore_root = "/tmp/kd/raiz"
trash_dir = "/tmp/kd/raiz/lixeira"
interface_da_pessoa = ["{PY}"]
[[callers]]
name = "agente"
max_level = "total"
allowed_paths = ["~"]
''')
subprocess.run("sudo -n rm -rf /tmp/kd; sudo -n mkdir -p /tmp/kd/raiz/etc; sudo -n sh -c 'nohup /tmp/kalangod servir --config /tmp/k.toml > /tmp/kd.log 2>&1 &'", shell=True)
for _ in range(100):
    if os.path.exists(SOCK): break
    time.sleep(0.1)
print(open("/tmp/kd.log").read().strip() if os.access("/tmp/kd.log", os.R_OK) else subprocess.run("sudo -n cat /tmp/kd.log", shell=True, capture_output=True, text=True).stdout.strip())
docs = f"{H}/Documentos"

# 1. who the command is: the person, no privileges, systemd mode, its own /dev/shm
open("/dev/shm/kalango-marcador-da-pessoa", "w").write("x")
v = sh("id -u; grep -E '^(NoNewPrivs|CapEff)' /proc/self/status | tr '\\t' ' '; stat -f -c %T /dev/shm; ls -A /dev/shm | wc -l; echo $TMPDIR; cat /proc/self/cgroup", docs)
print("      " + json.dumps(v)[:900])
out = v.get("saida", "")
check(v.get("modo") == "systemd", "kalangod como root usa o systemd-run (modo systemd)", v.get("modo"))
check(out.startswith("1000\n"), "o comando roda como a pessoa (uid 1000)", out)
check("NoNewPrivs: 1" in out and "CapEff: 0000000000000000" in out, "sem privilégios (no_new_privs, nenhuma capacidade)", out)
lines = out.splitlines()
check(len(lines) > 4 and lines[3] == "tmpfs" and lines[4] == "0", "/dev/shm próprio: tmpfs novo, vazio (o marcador da pessoa não aparece)", out)
check("kalango-agentes.slice" in out, "no cgroup dos agentes", out)

# 2. writing inside and outside
v = sh("echo ok > dentro.txt && echo dentro", docs)
check(v.get("codigo") == 0 and open(f"{docs}/dentro.txt").read() == "ok\n", "escreve em ~/Documentos", json.dumps(v)[:300])
before = open(f"{H}/.bashrc").read()
v = sh(f"echo EVIL >> {H}/.bashrc", docs)
check(v.get("codigo") != 0 and open(f"{H}/.bashrc").read() == before, "não escreve em ~/.bashrc (fora das pastas)", v.get("erros"))
v = sh(f"echo x > {H}/novo.txt", docs)
check(v.get("codigo") != 0 and not os.path.exists(f"{H}/novo.txt"), "não cria arquivo na raiz da pasta pessoal", v.get("erros"))

# 3. multiprocessing with the private /dev/shm
r = call("tarefas.executar", {"programa": "python3", "argumentos": ["-c", "import multiprocessing as m; m.Lock(); from concurrent.futures import ProcessPoolExecutor as P; print(list(P(2).map(abs,[-1])))"], "pasta": docs})
v = r.get("structuredContent", {})
check(v.get("codigo") == 0 and v.get("saida") == "[1]\n", "multiprocessing.Lock e ProcessPoolExecutor funcionam", (v.get("erros") or "")[-1500:])
v = sh("ls -la /dev/shm; grep shm /proc/self/mountinfo; python3 -c 'import multiprocessing as m; m.Lock(); print(\"lock ok\")' 2>&1 | tail -3", docs)
print("      shm: " + (v.get("saida") or "") + (v.get("erros") or ""))

# 4. the cgroup kill: a detached child does not outlive the task
v = sh("setsid sleep 60 < /dev/null > /dev/null 2>&1 & echo $! > fica.pid; sleep 0.3", docs)
pid = open(f"{docs}/fica.pid").read().strip()
time.sleep(1)
alive = os.path.exists(f"/proc/{pid}")
check(pid.isdigit() and not alive, f"o filho destacado (pid {pid}) morre com a tarefa", json.dumps(v)[:200])

# 5b. local sockets: other programs' are out of reach (Landlock ABI 9 + the abstract scope, or no AF_UNIX at all)
abi = subprocess.run(["python3", "-c", "import ctypes; print(ctypes.CDLL(None).syscall(444, 0, 0, 1))"], capture_output=True, text=True).stdout.strip()
print(f"      Landlock ABI do kernel: {abi}")
lst = socket.socket(socket.AF_UNIX); lst.bind("\0kalango-teste-abstrato"); lst.listen(1)
v = sh("python3 - <<'P'\nimport socket, os\nfor name, addr in [('bus', '/run/user/%d/bus' % os.getuid()), ('abstrato', '\\0kalango-teste-abstrato')]:\n    try:\n        s = socket.socket(socket.AF_UNIX); s.connect(addr); print(name, 'CONECTOU')\n    except OSError as e:\n        print(name, 'recusado', e.errno)\ntry:\n    p = os.environ['TMPDIR'] + '/s'; a = socket.socket(socket.AF_UNIX); a.bind(p); a.listen(1); b = socket.socket(socket.AF_UNIX); b.connect(p); print('proprio ok')\nexcept OSError as e:\n    print('proprio recusado', e.errno)\nP", docs)
out = (v.get("saida") or "") + (v.get("erros") or "")
check("CONECTOU" not in out and "bus recusado" in out and "abstrato recusado" in out, "o barramento da pessoa e um socket abstrato de fora: recusados", out.replace("\n", " | "))
if abi.isdigit() and int(abi) >= 9:
    check("proprio ok" in out, "um socket do próprio TMPDIR funciona (Landlock ABI 9)", out.replace("\n", " | "))
lst.close()

# 5. sem_unix: no local sockets, no D-Bus, no systemd
v = sh("python3 -c 'import socket; socket.socket(socket.AF_UNIX)' 2>&1; echo py=$?; systemd-run --user true 2>&1; echo sr=$?; busctl --user call org.freedesktop.systemd1 /org/freedesktop/systemd1 org.freedesktop.systemd1.Manager StartTransientUnit 'ssa(sv)a(sa(sv))' x.scope fail 0 0 2>&1; echo bc=$?", docs)
out = v.get("saida", "")
check("sr=0" not in out and "bc=0" not in out, "systemd-run --user e busctl recusados", out.replace("\n", " | ")[:500])

# 6. per-grant caches
v1 = sh("echo $CARGO_HOME; stat -c %U $CARGO_HOME; printf '[build]\\nrustc-wrapper = \"/tmp/x\"\\n' > $CARGO_HOME/config.toml; echo cfg=$?", docs)
a = approve_next(always=True)
r = call("tarefas.pedir_pasta", {"pasta": "~/Projetos/site"}); a.join()
check(r.get("structuredContent", {}).get("sempre") is True, "pasta de projeto aprovada (sempre)", json.dumps(r)[:300])
v2 = sh("echo $CARGO_HOME; echo x > index.html && echo escreveu", f"{H}/Projetos/site")
c1, c2 = v1.get("saida", "").split("\n")[0], v2.get("saida", "").split("\n")[0]
check(c1 != c2 and c1.startswith(f"{H}/.cache/kalango-agentes/") and c2.startswith(f"{H}/.cache/kalango-agentes/"), "um cache por concessão (antes e depois da aprovação)", f"{c1} | {c2}")
check(v1.get("saida", "").split("\n")[1] == os.environ.get("USER", "kalango") and "cfg=0" not in v1.get("saida", ""), "o cache é da pessoa, e o config.toml dele é só leitura", v1.get("saida", "").replace("\n", " | "))
check("escreveu" in v2.get("saida", ""), "escreve na pasta de projeto aprovada", json.dumps(v2)[:300])

# 7. person-only methods
cache2 = os.path.dirname(c2)
r = subprocess.run([PY, "-c", f"import json,socket;s=socket.socket(socket.AF_UNIX);s.connect('{SOCK}');f=s.makefile('rw');f.write(json.dumps({{'jsonrpc':'2.0','id':1,'method':'kalango/pastas_remover','params':{{'caminho':'~/Projetos/site'}}}})+'\\n');f.flush();print(f.readline())"],
    env={**os.environ, "LD_PRELOAD": "/lib/x86_64-linux-gnu/libm.so.6"}, capture_output=True, text=True)
check('"error"' in r.stdout, "a interface com LD_PRELOAD é recusada", r.stdout.strip()[:200])
r = subprocess.run(f"sudo -n systemd-run --quiet --scope --slice=kalango-agentes.slice --uid={os.getuid()} {PY} -c \"import json,socket;s=socket.socket(socket.AF_UNIX);s.connect('{SOCK}');f=s.makefile('rw');f.write(json.dumps({{'jsonrpc':'2.0','id':1,'method':'kalango/pastas_remover','params':{{'caminho':'~/Projetos/site'}}}})+'\\\\n');f.flush();print(f.readline())\"", shell=True, capture_output=True, text=True)
check('"error"' in r.stdout, "do cgroup dos agentes é recusada", (r.stdout + r.stderr).strip()[:300])
print("      " + subprocess.run(f"stat -c '%a %U:%G %n' {PY} /usr/bin /usr; ls -la --time-style=+ /usr/bin/python3*", shell=True, capture_output=True, text=True).stdout.replace(chr(10), ' | '))
print("      ambiente do roteiro: " + " ".join(k for k in os.environ if k.startswith(("LD_", "GIO_", "GDK_PIXBUF"))))
r = rpc("kalango/pastas_remover", {"caminho": "~/Projetos/site"})
print("      " + subprocess.run("sudo -n grep -a recusada /tmp/kd.log | tail -4", shell=True, capture_output=True, text=True).stdout)
check(r.get("result", {}).get("removida") is True and r.get("result", {}).get("caches_removidos") == 1, "a interface listada remove a pasta, e o cache da concessão vai junto", json.dumps(r)[:300])
check(not os.path.exists(cache2) and os.path.exists(os.path.dirname(c1)), "o cache da concessão removida sumiu; o das pastas de trabalho ficou", f"{cache2} {os.path.exists(cache2)}")
v = sh("echo y > index2.html", f"{H}/Projetos/site")
check(v.get("codigo") != 0, "removida, a pasta volta a ser só leitura", v.get("erros"))

# 8. the task's temporary folder is gone, and nothing is left running in the agents' slice
time.sleep(1)
left = subprocess.run("ls -d /tmp/kalango-tarefa-* 2>/dev/null | wc -l; systemctl list-units --type=scope --no-legend 'kalango-tarefa-*' | wc -l", shell=True, capture_output=True, text=True).stdout.split()
check(left == ["0", "0"], "nenhuma pasta temporária nem escopo de tarefa ficou", str(left))
# 9. connectors (run by `kalangod conector rodar`, as the person): own sockets and the one the manifest names; never
# the session bus nor systemd
os.makedirs("/tmp/conector", exist_ok=True)
subprocess.run(["cp", "/tmp/conector_teste.py", "/tmp/conector/"])
banco = socket.socket(socket.AF_UNIX); banco.bind("/tmp/kd-banco.sock"); banco.listen(1)
outro = socket.socket(socket.AF_UNIX); outro.bind("/tmp/kd-outro.sock"); outro.listen(1)
def manifest(extra):
    open("/tmp/conector.toml", "w").write(f'nome = "teste"\ncomando = "/usr/bin/python3"\nargumentos = ["-I", "/tmp/conector/conector_teste.py"]\npasta = "/tmp/conector"\n{extra}\n')
    return "/tmp/conector.toml"
r = subprocess.run([KD, "conector", "testar", manifest(f'sockets = ["/run/user/{os.getuid()}/bus"]')], capture_output=True, text=True)
check(r.returncode != 0 and "isolamento" in (r.stdout + r.stderr), "um manifesto que pede o barramento da sessão é recusado", (r.stdout + r.stderr).strip()[-300:])
p = subprocess.Popen([KD, "conector", "rodar", manifest('sockets = ["/tmp/kd-banco.sock"]')], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
def mcp(method, params, n=[0]):
    n[0] += 1
    p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": n[0], "method": method, "params": params}) + "\n"); p.stdin.flush()
    return json.loads(p.stdout.readline())
mcp("initialize", {"protocolVersion": "2025-06-18"})
script = f"import socket\nfor a in ['/run/user/{os.getuid()}/bus', '/tmp/kd-outro.sock', '/tmp/kd-banco.sock']:\n    try:\n        s = socket.socket(socket.AF_UNIX); s.connect(a); print('conectou', a)\n    except OSError as e:\n        print('recusado', a, e.errno)\n"
out = mcp("tools/call", {"name": "executar", "arguments": {"programa": ["/usr/bin/python3", "-c", script]}})["result"]["content"][0]["text"]
check("recusado /run/user" in out and "recusado /tmp/kd-outro.sock" in out and "conectou /tmp/kd-banco.sock" in out,
      "o conector alcança só o socket do manifesto (o barramento e outro socket: recusados)", out.replace("\n", " | "))
sr = f"XDG_RUNTIME_DIR=/run/user/{os.getuid()} DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/{os.getuid()}/bus systemd-run --user --quiet --scope /bin/true 2>&1; echo codigo=$?"
out = mcp("tools/call", {"name": "executar", "arguments": {"programa": ["/bin/sh", "-c", sr]}})["result"]["content"][0]["text"]
check("codigo=" in out and "codigo=0" not in out, "o conector não roda systemd-run --user", out.replace("\n", " | ")[-300:])
p.stdin.close(); p.terminate()

# 10. Moli, the agents' browser, under its own plan (Landlock: its folder and the proxy's port; no other socket)
moli = next((m for m in ["/usr/lib/kalango/moli"] if os.path.exists(m)), None)
if moli:
    os.makedirs("/tmp/moli-site", exist_ok=True); os.makedirs("/tmp/moli-dir", exist_ok=True)
    open("/tmp/moli-site/index.html", "w").write("<html><body><h1>ola moli</h1><script>document.body.append('scripts ok')</script></body></html>")
    http = subprocess.Popen(["python3", "-m", "http.server", "18491", "--bind", "127.0.0.1", "--directory", "/tmp/moli-site"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    plano = {"landlock": {"leitura": ["/usr", "/lib", "/lib64", "/bin", "/etc", "/proc", "/sys", "/dev", os.path.dirname(moli)], "escrita": ["/tmp/moli-dir", "/dev/null", "/dev/shm"],
             "listar": [], "portas": [18491], "fixas": [], "sockets": []}, "seccomp": {"sem_rede": False, "sem_unix": True},
             "ambiente": {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "HOME": "/tmp/moli-dir", "TMPDIR": "/tmp/moli-dir", "XDG_CACHE_HOME": "/tmp/moli-dir"},
             "programa": moli, "argumentos": ["fetch", "--wait", "networkidle", "--timeout-ms", "15000", "http://127.0.0.1:18491/"], "pasta": "/tmp/moli-dir"}
    r = subprocess.run([KD, "__tarefa", json.dumps(plano)], capture_output=True, text=True, timeout=120)
    check("ola moli" in r.stdout and "scripts ok" in r.stdout, "o Moli abre uma página (com scripts) com o perfil novo (sem sockets locais de fora)", (r.stdout[-300:] + r.stderr[-300:]).replace("\n", " | "))
    http.terminate()
else:
    print("      (o Moli não está nesta imagem: /usr/lib/kalango/moli)")

subprocess.run("sudo -n pkill -f '/tmp/kalangod servir'", shell=True)
print("TUDO OK" if ok_all else "ALGO FALHOU", flush=True)
sys.exit(0 if ok_all else 1)
