#!/usr/bin/python3
"""Screen sharing in the session test. The nested niri cannot cast (no GBM device), so a stand-in plays its
org.gnome.Mutter.ScreenCast, and another plays xdg-desktop-portal asking the shell's ScreenCast backend:

  compartilhar-teste.py mutter LOG               the compositor's side: sessions, RecordMonitor/RecordWindow, Start
                                                 (PipeWireStreamAdded 77 shortly after), Stop (Closed); calls in LOG
  compartilhar-teste.py pedir SAIDA APP_ID       the portal's side: CreateSession, SelectSources (monitor or window),
                                                 Start (the chooser shows); writes "resposta N" and the streams to SAIDA
  compartilhar-teste.py fechar                   Session.Close on that session (the app stops sharing)

Run inside the test session (its bus)."""
import sys

from gi.repository import Gio, GLib

BACKEND = "org.freedesktop.impl.portal.desktop.kalango"
SESSAO = "/org/freedesktop/portal/desktop/session/1_1/teste"

XML = """<node>
  <interface name="org.gnome.Mutter.ScreenCast">
    <method name="CreateSession"><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method>
  </interface>
  <interface name="org.gnome.Mutter.ScreenCast.Session">
    <method name="RecordMonitor"><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method>
    <method name="RecordWindow"><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method>
    <method name="Start"/>
    <method name="Stop"/>
    <signal name="Closed"/>
  </interface>
  <interface name="org.gnome.Mutter.ScreenCast.Stream">
    <signal name="PipeWireStreamAdded"><arg type="u"/></signal>
    <property name="Parameters" type="a{sv}" access="read"/>
  </interface>
</node>"""


def mutter(log):
    info = Gio.DBusNodeInfo.new_for_xml(XML)
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    estado = {"n": 0, "streams": {}}

    def escrever(t):
        with open(log, "a") as f:
            f.write(t + "\n")

    def stream_call(conn, sender, path, iface, method, params, inv):
        inv.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method)

    def stream_prop(conn, sender, path, iface, prop):
        return GLib.Variant("a{sv}", {"position": GLib.Variant("(ii)", (0, 0)), "size": GLib.Variant("(ii)", estado["streams"].get(path, (1, 1)))})

    def session_call(conn, sender, path, iface, method, params, inv):
        if method in ("RecordMonitor", "RecordWindow"):
            estado["n"] += 1
            sp = "/org/gnome/Mutter/ScreenCast/Stream/u%d" % estado["n"]
            if method == "RecordMonitor":
                escrever("RecordMonitor %s" % params.unpack()[0])
                estado["streams"][sp] = (1280, 800)
            else:
                props = params.unpack()[0]
                escrever("RecordWindow %s" % props.get("window-id"))
                estado["streams"][sp] = (1, 1)
            conn.register_object(sp, info.lookup_interface("org.gnome.Mutter.ScreenCast.Stream"), stream_call, stream_prop, None)
            estado.setdefault("por_sessao", {}).setdefault(path, []).append(sp)
            inv.return_value(GLib.Variant("(o)", (sp,)))
        elif method == "Start":
            escrever("Start")
            inv.return_value(None)

            def emitir():
                for sp in estado.get("por_sessao", {}).get(path, []):
                    conn.emit_signal(None, sp, "org.gnome.Mutter.ScreenCast.Stream", "PipeWireStreamAdded", GLib.Variant("(u)", (77,)))
                return False
            GLib.timeout_add(300, emitir)
        elif method == "Stop":
            escrever("Stop")
            inv.return_value(None)
            conn.emit_signal(None, path, "org.gnome.Mutter.ScreenCast.Session", "Closed", None)

    def main_call(conn, sender, path, iface, method, params, inv):
        estado["n"] += 1
        sp = "/org/gnome/Mutter/ScreenCast/Session/u%d" % estado["n"]
        conn.register_object(sp, info.lookup_interface("org.gnome.Mutter.ScreenCast.Session"), session_call, None, None)
        escrever("CreateSession")
        inv.return_value(GLib.Variant("(o)", (sp,)))

    bus.register_object("/org/gnome/Mutter/ScreenCast", info.lookup_interface("org.gnome.Mutter.ScreenCast"), main_call, None, None)
    Gio.bus_own_name_on_connection(bus, "org.gnome.Mutter.ScreenCast", Gio.BusNameOwnerFlags.REPLACE, lambda *a: escrever("registrado"), None)
    GLib.MainLoop().run()


def chamar(metodo, args, assinatura, timeout=10000):
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    return bus.call_sync(BACKEND, "/org/freedesktop/portal/desktop", "org.freedesktop.impl.portal.ScreenCast", metodo, GLib.Variant(assinatura, args),
                         GLib.VariantType("(ua{sv})"), Gio.DBusCallFlags.NONE, timeout, None).unpack()


def pedir(saida, app_id):
    req = "/org/freedesktop/portal/desktop/request/1_1/t%d"
    try:
        chamar("CreateSession", (req % 1, SESSAO, app_id, {}), "(oosa{sv})")
        chamar("SelectSources", (req % 2, SESSAO, app_id, {"types": GLib.Variant("u", 3), "cursor_mode": GLib.Variant("u", 2)}), "(oosa{sv})")
        r, res = chamar("Start", (req % 3, SESSAO, app_id, "", {}), "(oossa{sv})", 120000)
        streams = res.get("streams", [])
        texto = "resposta %d\n" % r + "".join("stream %d %s %s\n" % (n, p.get("source_type"), p.get("size")) for n, p in streams)
    except GLib.Error as e:
        texto = "erro %s\n" % e.message
    with open(saida, "w") as f:
        f.write(texto)


def fechar():
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    bus.call_sync(BACKEND, SESSAO, "org.freedesktop.impl.portal.Session", "Close", None, None, Gio.DBusCallFlags.NONE, 10000, None)


def main():
    a = sys.argv[1:]
    if a[:1] == ["mutter"]:
        mutter(a[1])
    elif a[:1] == ["pedir"]:
        pedir(a[1], a[2])
    elif a[:1] == ["fechar"]:
        fechar()
    else:
        print(__doc__, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
