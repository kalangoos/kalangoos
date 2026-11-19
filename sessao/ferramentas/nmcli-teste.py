#!/usr/bin/env python3
"""A stand-in for nmcli in the session test (KALANGO_NMCLI): the networks of a home — the connected one, a saved one,
an open café, a neighbour's — kept in a JSON file (KALANGO_NMCLI_ESTADO), so the Settings' Network page and kalangod
can be driven without ever touching the computer's real NetworkManager. Every command is appended to
KALANGO_NMCLI_ESTADO + ".log". Only what KalangoOS asks is answered: the terse lists (-t -f …), -g single values,
connection modify / delete / up / down, device wifi connect (a protected network wants the password "senha-certa"), radio."""
import json
import os
import sys

ESTADO = os.environ.get("KALANGO_NMCLI_ESTADO", "/tmp/nmcli-teste.json")

INICIAL = {
    "wifi": True,
    "redes": [
        {"ssid": "Lunar-Casa_5G", "sinal": 82, "seguranca": "WPA3", "taxa": "866 Mbit/s", "freq": "5180 MHz"},
        {"ssid": "Lunar-Casa", "sinal": 64, "seguranca": "WPA2", "taxa": "270 Mbit/s", "freq": "2437 MHz"},
        {"ssid": "Café Lunar", "sinal": 58, "seguranca": "", "taxa": "130 Mbit/s", "freq": "2412 MHz"},
        {"ssid": "Vizinho 2.4", "sinal": 31, "seguranca": "WPA2", "taxa": "65 Mbit/s", "freq": "2462 MHz"},
    ],
    "conexoes": [
        {"nome": "Lunar-Casa_5G", "ssid": "Lunar-Casa_5G", "tipo": "802-11-wireless", "auto": True, "medida": False},
        {"nome": "Lunar-Casa", "ssid": "Lunar-Casa", "tipo": "802-11-wireless", "auto": True, "medida": False},
    ],
    "ativa": "Lunar-Casa_5G",
}


def ler():
    try:
        return json.load(open(ESTADO))
    except Exception:
        return json.loads(json.dumps(INICIAL))


def gravar(e):
    json.dump(e, open(ESTADO, "w"), ensure_ascii=False, indent=1)


def esc(v):
    return str(v).replace("\\", "\\\\").replace(":", "\\:")


def linhas(registros, campos):
    for r in registros:
        print(":".join(esc(r.get(c, "")) for c in campos))


def main(a):
    with open(ESTADO + ".log", "a") as f:
        f.write(" ".join(a) + "\n")
    e = ler()
    campos = []
    valor = None
    while a and a[0].startswith("-"):
        o = a.pop(0)
        if o == "-f":
            campos = a.pop(0).split(",")
        elif o == "-g":
            valor = a.pop(0)
    ativa = e.get("ativa")
    if a[:1] == ["radio"]:
        if len(a) >= 3:
            e["wifi"] = a[2] == "on" if a[1] == "wifi" else e["wifi"]
            if not e["wifi"]:
                e["ativa"] = None
            gravar(e)
            return 0
        linhas([{"WIFI": "enabled" if e["wifi"] else "disabled", "WWAN": "disabled"}], campos)
        return 0
    if a[:1] == ["general"]:
        linhas([{"NETWORKING": "enabled", "STATE": "connected" if ativa else "disconnected"}], campos)
        return 0
    if a[:2] == ["device", "wifi"] and a[2:3] == ["list"]:
        if not e["wifi"]:
            return 0
        regs = []
        for r in e["redes"]:
            ssid = r["ssid"]
            em_uso = any(c["nome"] == ativa and c["ssid"] == ssid for c in e["conexoes"])
            regs.append({"IN-USE": "*" if em_uso else " ", "SSID": ssid, "SIGNAL": r["sinal"], "SECURITY": r["seguranca"], "RATE": r["taxa"], "FREQ": r["freq"]})
        linhas(regs, campos)
        return 0
    if a[:3] == ["device", "wifi", "connect"]:
        ssid = a[3]
        rede = next((r for r in e["redes"] if r["ssid"] == ssid), None)
        if rede is None:
            print("Error: No network with SSID '%s' found." % ssid, file=sys.stderr)
            return 10
        senha = a[a.index("password") + 1] if "password" in a else None
        if rede["seguranca"] and senha != "senha-certa":
            print("Error: Connection activation failed: Secrets were required, but not provided.", file=sys.stderr)
            return 4
        if not any(c["nome"] == ssid for c in e["conexoes"]):
            e["conexoes"].append({"nome": ssid, "ssid": ssid, "tipo": "802-11-wireless", "auto": True, "medida": False})
        e["ativa"] = ssid
        gravar(e)
        print("Device 'wlan0' successfully activated.")
        return 0
    if a[:1] == ["device"] and a[1:2] in (["show"],):
        if valor == "IP4.ADDRESS":
            print("192.168.0.42/24" if (a[2] == "wlan0" and ativa) else "")
        return 0
    if a[:1] == ["device"] and len(a) == 1 or a[:2] == ["device", "status"]:
        linhas([{"DEVICE": "wlan0", "TYPE": "wifi", "STATE": "connected" if ativa else "disconnected", "CONNECTION": ativa or "--"},
                {"DEVICE": "eth0", "TYPE": "ethernet", "STATE": "unavailable", "CONNECTION": "--"},
                # a container's interface and its bridge (Docker's): not ports of this computer
                {"DEVICE": "veth9f3a1c", "TYPE": "ethernet", "STATE": "unmanaged", "CONNECTION": "--"},
                {"DEVICE": "docker0", "TYPE": "bridge", "STATE": "connected (externally)", "CONNECTION": "docker0"}], campos)
        return 0
    if a[:1] == ["connection"]:
        sub = a[1] if len(a) > 1 else "show"
        nome = a[2] if len(a) > 2 else None
        if sub == "show" and (nome is None or nome == "--active"):
            regs = [{"NAME": c["nome"], "TYPE": c["tipo"], "AUTOCONNECT": "yes" if c["auto"] else "no", "ACTIVE": "yes" if c["nome"] == ativa else "no",
                     "DEVICE": "wlan0" if c["nome"] == ativa else "--"} for c in e["conexoes"]]
            if nome == "--active":
                regs = [r for r in regs if r["ACTIVE"] == "yes"]
            linhas(regs, campos)
            return 0
        c = next((x for x in e["conexoes"] if x["nome"] == nome), None)
        if c is None:
            print("Error: unknown connection '%s'." % nome, file=sys.stderr)
            return 10
        if sub == "show":
            print({"802-11-wireless.ssid": c["ssid"], "connection.metered": "yes" if c["medida"] else "no",
                   "connection.autoconnect": "yes" if c["auto"] else "no"}.get(valor, ""))
        elif sub == "modify":
            chave, v = a[3], a[4]
            if chave == "connection.autoconnect":
                c["auto"] = v == "yes"
            elif chave == "connection.metered":
                c["medida"] = v == "yes"
            gravar(e)
        elif sub == "delete":
            e["conexoes"].remove(c)
            if ativa == nome:
                e["ativa"] = None
            gravar(e)
        elif sub == "up":
            e["ativa"] = nome
            gravar(e)
        elif sub == "down":
            if ativa == nome:
                e["ativa"] = None
            gravar(e)
        return 0
    print("nmcli-teste: unsupported: %s" % " ".join(a), file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
