#!/usr/bin/python3
"""The portal frontend's part in the session test (xdg-desktop-portal asks the desktop's Access backend, then keeps the
answer in the permission store). Run inside the test session (its bus):

  acesso-teste.py pedir SAIDA APP_ID ICONE   AccessDialog on the shell's backend; writes "resposta N" to SAIDA
  acesso-teste.py guardar APP_ID             stores "yes" for the microphone, as the frontend does after a yes
  acesso-teste.py consultar SAIDA            writes the apps the microphone entry has (one per line) to SAIDA

The permission store is the test profile's (the test bus runs with its own XDG_DATA_HOME)."""
import sys

from gi.repository import Gio, GLib

BACKEND = "org.freedesktop.impl.portal.desktop.kalango"
STORE = ("org.freedesktop.impl.portal.PermissionStore", "/org/freedesktop/impl/portal/PermissionStore", "org.freedesktop.impl.portal.PermissionStore")


def bus():
    return Gio.bus_get_sync(Gio.BusType.SESSION, None)


def pedir(saida, app_id, icone):
    try:
        r = bus().call_sync(BACKEND, "/org/freedesktop/portal/desktop", "org.freedesktop.impl.portal.Access", "AccessDialog",
                            GLib.Variant("(osssssa{sv})", ("/org/freedesktop/portal/desktop/request/1_1/teste", app_id, "", "Turn On Microphone?", "", "",
                                                           {"icon": GLib.Variant("s", icone)})),
                            GLib.VariantType("(ua{sv})"), Gio.DBusCallFlags.NONE, 120000, None)
        texto = "resposta %d\n" % r.unpack()[0]
    except GLib.Error as e:
        texto = "erro %s\n" % e.message
    with open(saida, "w") as f:
        f.write(texto)


def guardar(app_id):
    bus().call_sync(STORE[0], STORE[1], STORE[2], "SetPermission", GLib.Variant("(sbssas)", ("devices", True, "microphone", app_id, ["yes"])),
                    None, Gio.DBusCallFlags.NONE, 10000, None)


def consultar(saida):
    try:
        r = bus().call_sync(STORE[0], STORE[1], STORE[2], "Lookup", GLib.Variant("(ss)", ("devices", "microphone")),
                            GLib.VariantType("(a{sas}v)"), Gio.DBusCallFlags.NONE, 10000, None)
        apps = sorted(r.unpack()[0].keys())
    except GLib.Error:
        apps = []
    with open(saida, "w") as f:
        f.write("".join(a + "\n" for a in apps))


def main():
    a = sys.argv[1:]
    if a[:1] == ["pedir"]:
        pedir(a[1], a[2], a[3])
    elif a[:1] == ["guardar"]:
        guardar(a[1])
    elif a[:1] == ["consultar"]:
        consultar(a[1])
    else:
        print(__doc__, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
