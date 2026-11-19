#!/usr/bin/python3
"""The file chooser asked for by an app that is not KalangoOS's own (a browser, an office suite) in the session test:
plays xdg-desktop-portal calling the shell's FileChooser backend, as it does for those apps (KalangoOS's own apps call
the chooser in the shell directly).

  seletor-teste.py abrir SAIDA APP_ID     OpenFile with a "*.txt" filter; writes "resposta N" and the chosen URIs to
                                          SAIDA (0: chosen, 1: cancelled)

Run inside the test session (its bus)."""
import sys

from gi.repository import Gio, GLib

BACKEND = "org.freedesktop.impl.portal.desktop.kalango"
PATH = "/org/freedesktop/portal/desktop"
HANDLE = "/org/freedesktop/portal/desktop/request/1_1/seletorteste"


def abrir(saida, app_id):
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    filtros = GLib.Variant("a(sa(us))", [("Documentos de texto", [(0, "*.txt")])])
    args = GLib.Variant("(osssa{sv})", (HANDLE, app_id, "", "Abrir arquivo", {"filters": filtros}))
    r = bus.call_sync(BACKEND, PATH, "org.freedesktop.impl.portal.FileChooser", "OpenFile", args,
                      GLib.VariantType("(ua{sv})"), Gio.DBusCallFlags.NONE, 180000, None)
    resposta, resultado = r.unpack()
    with open(saida, "w") as f:
        f.write(f"resposta {resposta}\n")
        for u in resultado.get("uris", []):
            f.write(u + "\n")


if __name__ == "__main__":
    if len(sys.argv) == 4 and sys.argv[1] == "abrir":
        abrir(sys.argv[2], sys.argv[3])
    else:
        sys.exit(__doc__)
