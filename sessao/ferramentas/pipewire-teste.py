#!/usr/bin/env python3
"""A stand-in for PipeWire's commands in the session test, by the name it is called with (symlinks): pw-dump-teste,
wpctl-teste, pw-cli-teste, pw-record-teste and pipewire-teste (`pipewire -c FILE`: a filter as a client of its own —
the spatial audio's sink, there while the process runs). A home's sound: a Bluetooth headset (in use), the speakers, a monitor's
HDMI; a USB microphone (in use) and the headset's; a music app, a browser and a call app playing. Kept in KALANGO_PW_ESTADO (JSON), each command
appended to that file + ".log". pw-record gives a 440 Hz tone (the level meter moves) — the computer's real
microphone is never opened."""
import json
import math
import os
import re
import signal
import struct
import sys
import time

ESTADO = os.environ.get("KALANGO_PW_ESTADO", "/tmp/pipewire-teste.json")

INICIAL = {
    "saidas": [
        {"id": 60, "nome": "bluez_output.C8_24_78", "desc": "Fone Lunar", "api": "bluez5", "codec": "ldac", "forma": "headset", "vol": 0.62, "canais": [0.24, 0.24]},
        {"id": 61, "nome": "alsa_output.pci-analog-stereo", "desc": "Alto-falantes", "api": "alsa", "vol": 0.5, "canais": [0.125, 0.125]},
        {"id": 62, "nome": "alsa_output.pci-hdmi-stereo", "desc": "Monitor MSI", "api": "alsa", "vol": 0.8, "canais": [0.5, 0.5]},
    ],
    "entradas": [
        {"id": 70, "nome": "alsa_input.usb-yeti", "desc": "Microfone USB", "api": "alsa", "bus": "usb", "vol": 0.9},
        {"id": 71, "nome": "bluez_input.C8_24_78", "desc": "Fone Lunar", "api": "bluez5", "forma": "headset", "vol": 0.7},
    ],
    "apps": [{"id": 80, "app": "Música", "icone": "multimedia-audio-player", "vol": 0.75},
             {"id": 81, "app": "Chrome", "icone": "google-chrome", "vol": 0.5},
             {"id": 82, "app": "Discord", "icone": "discord", "vol": 0.62}],
    "saida": "bluez_output.C8_24_78",
    "entrada": "alsa_input.usb-yeti",
}


def ler():
    try:
        return json.load(open(ESTADO))
    except Exception:
        return json.loads(json.dumps(INICIAL))


def gravar(e):
    json.dump(e, open(ESTADO, "w"), ensure_ascii=False, indent=1)


def todos(e):
    return e["saidas"] + e["entradas"] + e["apps"]


def espacial(e):
    """The spatial audio's filter, while its process runs."""
    f = e.get("espacial")
    if not f:
        return None
    try:
        os.kill(f["pid"], 0)
        return f
    except OSError:
        return None


def pw_dump(e):
    objs = [{"id": 40, "type": "PipeWire:Interface:Device", "info": {"props": {"device.api": "alsa", "device.bus": "usb"}}}]
    for s in e["saidas"] + e["entradas"]:
        p = {"media.class": "Audio/Sink" if s in e["saidas"] else "Audio/Source", "node.name": s["nome"], "node.description": s["desc"], "device.api": s["api"]}
        if s.get("codec"):
            p["api.bluez5.codec"] = s["codec"]
        if s.get("forma"):
            p["device.form_factor"] = s["forma"]
        if s.get("bus") == "usb":
            p["device.id"] = 40
        o = {"id": s["id"], "type": "PipeWire:Interface:Node", "info": {"props": p}}
        if s.get("canais"):
            o["info"]["params"] = {"Props": [{"channelVolumes": s["canais"]}]}
        objs.append(o)
    f = espacial(e)
    if f:
        objs.append({"id": 95, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Audio/Sink", "node.name": "kalango_espacial", "node.description": "Áudio espacial"}}})
        objs.append({"id": 96, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Stream/Output/Audio", "node.name": "kalango_espacial.saida", "target.object": f["alvo"]}}})
    for a in e["apps"]:
        objs.append({"id": a["id"], "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Stream/Output/Audio", "application.name": a["app"], "application.icon-name": a["icone"]}}})
    # who is capturing (the privacy chips): an app recording the microphone, one looking at the camera, one receiving
    # niri's screen cast — each consumer linked to its source, as PipeWire shows them
    fontes = {"camera": (130, {"media.class": "Video/Source", "node.name": "v4l2_input.usb-webcam", "node.description": "Webcam", "device.api": "v4l2"}),
              "tela": (131, {"media.class": "Video/Source", "node.name": "niri", "node.description": "niri screen cast"})}
    for c in e.get("capturas", []):
        if c["tipo"] == "microfone":
            o = {"id": c["id"], "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Stream/Input/Audio", "application.name": c["app"]}, "params": {"Props": [{"mute": bool(c.get("mudo"))}]}}}
            if c.get("chamada"):
                o["info"]["props"]["media.role"] = "Communication"
            objs.append(o)
            continue
        fid, fp = fontes[c["tipo"]]
        if not any(x["id"] == fid for x in objs):
            objs.append({"id": fid, "type": "PipeWire:Interface:Node", "info": {"props": fp}})
        objs.append({"id": c["id"], "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Stream/Input/Video", "application.name": c["app"]}}})
        objs.append({"id": c["id"] + 1000, "type": "PipeWire:Interface:Link", "info": {"output-node-id": fid, "input-node-id": c["id"]}})
    # (as the real pw-dump writes a metadata object: its name in the top-level "props", not in "info")
    objs.append({"id": 90, "type": "PipeWire:Interface:Metadata", "props": {"metadata.name": "default"},
                 "metadata": [{"key": "default.audio.sink", "value": {"name": e["saida"]}}, {"key": "default.audio.source", "value": {"name": e["entrada"]}}]})
    print(json.dumps(objs))


def alvo(e, x):
    if x == "@DEFAULT_AUDIO_SINK@" and e["saida"] == "kalango_espacial":
        return e.setdefault("espacial_no", {"id": 95, "vol": 1.0})
    if x == "95":
        return {"id": 95, "nome": "kalango_espacial", "vol": 1.0}
    if x == "@DEFAULT_AUDIO_SINK@":
        return next(s for s in e["saidas"] if s["nome"] == e["saida"])
    if x == "@DEFAULT_AUDIO_SOURCE@":
        return next(s for s in e["entradas"] if s["nome"] == e["entrada"])
    return next((s for s in todos(e) + e.get("capturas", []) if str(s["id"]) == x), None)


def wpctl(e, a):
    # (without "-l LIMIT")
    b, i = [], 0
    while i < len(a):
        if a[i] == "-l":
            i += 2
            continue
        b.append(a[i])
        i += 1
    a = b
    if a[0] == "get-volume":
        s = alvo(e, a[1])
        if s is None:
            return 1
        print("Volume: %.2f%s" % (s["vol"], " [MUTED]" if s.get("mudo") else ""))
    elif a[0] == "set-mute":
        # set-mute ID 1|0|toggle (the media keys)
        s = alvo(e, a[1])
        if s is None:
            return 1
        s["mudo"] = (not s.get("mudo")) if a[2] == "toggle" else a[2] == "1"
        gravar(e)
    elif a[0] == "set-volume":
        s = alvo(e, a[1])
        # "0.5", or relative as the media keys send it: "5%+" / "5%-"
        v = a[2]
        if v.endswith("+") or v.endswith("-"):
            passo = float(v[:-1].rstrip("%")) / (100.0 if "%" in v else 1.0)
            novo = s["vol"] + (passo if v.endswith("+") else -passo)
        else:
            novo = float(v.rstrip("%")) / (100.0 if v.endswith("%") else 1.0)
        s["vol"] = max(0.0, min(1.0, novo))
        if s.get("canais"):
            s["canais"] = [round(s["vol"] ** 3, 4)] * len(s["canais"])
        gravar(e)
    elif a[0] == "set-default":
        s = alvo(e, a[1])
        if s and s.get("nome") == "kalango_espacial":
            e["saida"] = "kalango_espacial"
        elif s in e["saidas"]:
            e["saida"] = s["nome"]
        elif s in e["entradas"]:
            e["entrada"] = s["nome"]
        gravar(e)
    return 0


def pw_cli(e, a):
    # destroy ID: a capture stream cut (the app sees its stream end)
    if a[0] == "destroy":
        e["capturas"] = [c for c in e.get("capturas", []) if str(c["id"]) != a[1]]
        gravar(e)
        return 0
    # set-param ID Props '{ channelVolumes: [ l, r ] }'
    s = alvo(e, a[1])
    texto = a[3]
    nums = texto[texto.index("[") + 1:texto.index("]")].split(",")
    s["canais"] = [float(n) for n in nums]
    gravar(e)
    return 0


def pw_record():
    saida = sys.stdout.buffer
    t = 0
    try:
        while True:
            bloco = b"".join(struct.pack("<h", int(16000 * math.sin(2 * math.pi * 440 * (t + i) / 8000))) for i in range(400))
            t += 400
            saida.write(bloco)
            saida.flush()
            time.sleep(0.05)
    except (BrokenPipeError, KeyboardInterrupt):
        return 0


def pipewire(a):
    """`pipewire -c FILE`: the filter runs until it is ended; its sink plays into the configuration's target."""
    conf = open(a[a.index("-c") + 1]).read()
    m = re.search(r'target.object = "([^"]*)"', conf)
    e = ler()
    e["espacial"] = {"alvo": m.group(1) if m else "", "pid": os.getpid()}
    gravar(e)

    def fim(*_):
        e = ler()
        e.pop("espacial", None)
        if e["saida"] == "kalango_espacial":
            e["saida"] = e["saidas"][0]["nome"]
        gravar(e)
        sys.exit(0)
    signal.signal(signal.SIGTERM, fim)
    while True:
        time.sleep(1)


def main():
    nome = os.path.basename(sys.argv[0]).replace("-teste", "")
    a = sys.argv[1:]
    with open(ESTADO + ".log", "a") as f:
        f.write(nome + " " + " ".join(a) + "\n")
    if nome == "pw-record":
        return pw_record()
    if nome == "pipewire":
        return pipewire(a)
    e = ler()
    # (the test's own: `pw-dump-teste --capturas JSON` sets who is capturing, the rest as it is)
    if nome == "pw-dump" and a[:1] == ["--capturas"]:
        e["capturas"] = json.loads(a[1])
        gravar(e)
        return 0
    if nome == "pw-dump":
        pw_dump(e)
        return 0
    if nome == "wpctl":
        return wpctl(e, a)
    if nome == "pw-cli":
        return pw_cli(e, a)
    print("pipewire-teste: unsupported %s" % nome, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
