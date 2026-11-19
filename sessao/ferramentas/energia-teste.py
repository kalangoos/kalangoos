#!/usr/bin/env python3
"""A stand-in for the power commands in the session test, by the name it is called with (symlinks):
powerprofilesctl-teste (power-profiles-daemon: get, list, set) and upower-teste (-e, -i: a laptop's battery at 82%,
charging, full in 35 minutes, health 96%). The mode is kept in KALANGO_ENERGIA_ESTADO (JSON), each command appended to
that file + ".log" — the computer's real power mode is never changed."""
import json
import os
import sys

ESTADO = os.environ.get("KALANGO_ENERGIA_ESTADO", "/tmp/energia-teste.json")
PERFIS = ["performance", "balanced", "power-saver"]


def ler():
    try:
        return json.load(open(ESTADO))
    except Exception:
        return {"perfil": "balanced"}


def powerprofilesctl(a):
    e = ler()
    if not a or a[0] == "get":
        print(e["perfil"])
    elif a[0] == "list":
        for p in PERFIS:
            print(("* " if p == e["perfil"] else "  ") + p + ":")
            print("    CpuDriver:\tintel_pstate")
            print("    Degraded:   no")
            print()
    elif a[0] == "set" and len(a) > 1 and a[1] in PERFIS:
        e["perfil"] = a[1]
        json.dump(e, open(ESTADO, "w"))
    else:
        return 1
    return 0


def upower(a):
    if a and a[0] == "-e":
        print("/org/freedesktop/UPower/devices/line_power_AC")
        print("/org/freedesktop/UPower/devices/battery_BAT0")
        print("/org/freedesktop/UPower/devices/DisplayDevice")
    elif a and a[0] == "-i" and len(a) > 1 and "battery_BAT0" in a[1]:
        # the state can say otherwise (the critical alert's test): {"bateria": {"pct": 5, "minutos": 6}} = on battery
        b = ler().get("bateria")
        if b:
            print("  native-path:          BAT0\n  vendor:               Kalango\n  power supply:         yes\n  battery\n"
                  "    present:             yes\n    rechargeable:        yes\n    state:               discharging\n"
                  "    energy:              2,8 Wh\n    time to empty:       %s,0 minutes\n    percentage:          %s%%\n"
                  "    capacity:            96,1%%\n    technology:          lithium-ion" % (b.get("minutos", 6), b.get("pct", 5)))
            return 0
        print("  native-path:          BAT0\n  vendor:               Kalango\n  power supply:         yes\n  battery\n"
              "    present:             yes\n    rechargeable:        yes\n    state:               charging\n"
              "    energy:              46,7 Wh\n    time to full:        35,0 minutes\n    percentage:          82%\n"
              "    capacity:            96,1%\n    technology:          lithium-ion")
    elif a and a[0] == "-i":
        print("  power supply:         yes\n  present:             yes\n  state:               charging\n  percentage:          82%")
    return 0


def main():
    nome = os.path.basename(sys.argv[0]).replace("-teste", "")
    a = sys.argv[1:]
    with open(ESTADO + ".log", "a") as f:
        f.write(nome + " " + " ".join(a) + "\n")
    if nome == "powerprofilesctl":
        return powerprofilesctl(a)
    if nome == "upower":
        return upower(a)
    print("energia-teste: unsupported %s" % nome, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
