#!/usr/bin/python3
"""A stand-in for greetd (the login daemon), for tests only: its protocol on a Unix socket (JSON with a 4-byte
native-endian length). usage: greetd-teste.py <socket> <log file> [password]

Any user gets a secret question; the password (default "kalango-teste") succeeds, anything else is an auth_error.
The user "convidado" needs no password. start_session is written to the log (one JSON line) and answered with
success, as greetd does before starting the session."""
import json, os, socket, struct, sys, threading

SOCK, LOG = sys.argv[1], sys.argv[2]
SENHA = sys.argv[3] if len(sys.argv) > 3 else "kalango-teste"


def ler(c):
    n = c.recv(4, socket.MSG_WAITALL)
    if len(n) < 4:
        return None
    (t,) = struct.unpack("=I", n)
    return json.loads(c.recv(t, socket.MSG_WAITALL))


def enviar(c, v):
    b = json.dumps(v).encode()
    c.sendall(struct.pack("=I", len(b)) + b)


def registrar(v):
    with open(LOG, "a") as f:
        f.write(json.dumps(v) + "\n")


def atender(c):
    usuario = None
    while True:
        v = ler(c)
        if v is None:
            return
        registrar(v if v.get("type") != "post_auth_message_response" else {"type": v["type"]})
        t = v.get("type")
        if t == "create_session":
            usuario = v.get("username")
            if usuario == "convidado":
                enviar(c, {"type": "success"})
            else:
                enviar(c, {"type": "auth_message", "auth_message_type": "secret", "auth_message": "Password: "})
        elif t == "post_auth_message_response":
            if v.get("response") == SENHA:
                enviar(c, {"type": "success"})
            else:
                enviar(c, {"type": "error", "error_type": "auth_error", "description": "pam_authenticate: Authentication failure"})
        else:
            enviar(c, {"type": "success"})


if os.path.exists(SOCK):
    os.unlink(SOCK)
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(SOCK)
s.listen(4)
while True:
    c, _ = s.accept()
    threading.Thread(target=atender, args=(c,), daemon=True).start()
