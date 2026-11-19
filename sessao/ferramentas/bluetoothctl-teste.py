#!/usr/bin/env python3
"""A stand-in for bluetoothctl in the session test (KALANGO_BLUETOOTHCTL): a controller with the person's devices — a
headset connected (80 %), a mouse connected, a phone paired but away — and, once a scan runs, a TV and a game
controller nearby. Kept in KALANGO_BLUETOOTHCTL_ESTADO (JSON); every command is appended to that file + ".log". The
computer's real Bluetooth is never touched."""
import json
import os
import sys

ESTADO = os.environ.get("KALANGO_BLUETOOTHCTL_ESTADO", "/tmp/bluetoothctl-teste.json")

INICIAL = {
    "ligado": True,
    "nome": "kalango-teste",
    "aparelhos": [
        {"mac": "C8:24:78:67:3D:CF", "nome": "Fone Lunar", "icone": "audio-headset", "pareado": True, "conectado": True, "bateria": 80},
        {"mac": "E1:59:FA:A1:C5:D8", "nome": "Mouse MX", "icone": "input-mouse", "pareado": True, "conectado": True, "bateria": 45},
        {"mac": "41:42:AF:8B:56:91", "nome": "Pixel de Ana", "icone": "phone", "pareado": True, "conectado": False},
    ],
    "perto": [
        {"mac": "34:FC:99:13:52:32", "nome": "TV da sala", "icone": "video-display", "pareado": False, "conectado": False},
        {"mac": "64:68:70:4E:6E:38", "nome": "Controle sem fio", "icone": "input-gaming", "pareado": False, "conectado": False},
    ],
    "procurou": False,
}


def ler():
    try:
        return json.load(open(ESTADO))
    except Exception:
        return json.loads(json.dumps(INICIAL))


def gravar(e):
    json.dump(e, open(ESTADO, "w"), ensure_ascii=False, indent=1)


def main(a):
    with open(ESTADO + ".log", "a") as f:
        f.write(" ".join(a) + "\n")
    e = ler()
    while a and a[0].startswith("--"):
        o = a.pop(0)
        if o in ("--timeout", "--agent"):
            a.pop(0)
    todos = e["aparelhos"] + (e["perto"] if e["procurou"] else [])
    acha = lambda mac: next((x for x in todos if x["mac"] == mac), None)
    cmd = a[0] if a else ""
    if cmd == "show":
        print("Controller A8:3B:76:7C:48:60 (public)\n\tName: %s\n\tAlias: %s\n\tPowered: %s\n\tDiscoverable: no" % (e["nome"], e["nome"], "yes" if e["ligado"] else "no"))
    elif cmd == "power":
        e["ligado"] = a[1] == "on"
        if not e["ligado"]:
            for x in e["aparelhos"]:
                x["conectado"] = False
        gravar(e)
        print("Changing power %s succeeded" % a[1])
    elif cmd == "scan":
        e["procurou"] = True
        gravar(e)
        print("Discovery started")
    elif cmd == "devices":
        filtro = a[1] if len(a) > 1 else None
        for x in todos:
            if filtro == "Paired" and not x["pareado"] or filtro == "Connected" and not x["conectado"]:
                continue
            print("Device %s %s" % (x["mac"], x["nome"]))
    elif cmd == "info":
        x = acha(a[1])
        if not x:
            print("Device %s not available" % a[1])
            return 1
        print("Device %s (public)\n\tName: %s\n\tAlias: %s\n\tIcon: %s\n\tPaired: %s\n\tTrusted: %s\n\tConnected: %s" % (
            x["mac"], x["nome"], x["nome"], x["icone"], "yes" if x["pareado"] else "no", "yes" if x["pareado"] else "no", "yes" if x["conectado"] else "no"))
        if x.get("bateria") is not None:
            print("\tBattery Percentage: 0x%02x (%d)" % (x["bateria"], x["bateria"]))
    elif cmd in ("pair", "trust", "connect", "disconnect", "remove"):
        x = acha(a[1])
        if not x:
            print("Device %s not available" % a[1])
            return 1
        if cmd == "pair":
            x["pareado"] = True
            if x in e["perto"]:
                e["perto"].remove(x)
                e["aparelhos"].append(x)
            print("Pairing successful")
        elif cmd == "connect":
            x["conectado"] = True
            print("Connection successful")
        elif cmd == "disconnect":
            x["conectado"] = False
            print("Successful disconnected")
        elif cmd == "remove":
            if x in e["aparelhos"]:
                e["aparelhos"].remove(x)
            print("Device has been removed")
        else:
            print("Changing %s trust succeeded" % a[1])
        gravar(e)
    else:
        print("bluetoothctl-teste: unsupported: %s" % " ".join(a), file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
