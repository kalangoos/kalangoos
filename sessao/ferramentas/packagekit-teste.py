#!/usr/bin/python3
"""A stand-in for PackageKit's offline-update API on the *session* bus, for tests only (the shell uses it when
KALANGO_PACKAGEKIT_BUS=session). usage: packagekit-teste.py <prepared packages> [log file]

UpdatePrepared is true when the count is above 0, GetPrepared returns that many package ids, and every Trigger
call is written to the log file (so a test can check that the update was asked for)."""
import sys
from gi.repository import Gio, GLib

COUNT = int(sys.argv[1]) if len(sys.argv) > 1 else 3
LOG = sys.argv[2] if len(sys.argv) > 2 else None

XML = """<node><interface name="org.freedesktop.PackageKit.Offline">
  <method name="GetPrepared"><arg type="as" direction="out"/></method>
  <method name="Trigger"><arg type="s" direction="in"/></method>
  <property name="UpdatePrepared" type="b" access="read"/>
</interface></node>"""


def call(conn, sender, path, iface, method, params, inv):
    if method == "GetPrepared":
        ids = [f"pacote-{i};1.0;amd64;kalango" for i in range(COUNT)]
        inv.return_value(GLib.Variant("(as)", (ids,)))
    elif method == "Trigger":
        if LOG:
            with open(LOG, "a") as f:
                f.write(f"Trigger {params.unpack()[0]}\n")
        inv.return_value(None)


def prop(conn, sender, path, iface, name):
    return GLib.Variant("b", COUNT > 0)


def acquired(conn, name):
    info = Gio.DBusNodeInfo.new_for_xml(XML)
    conn.register_object("/org/freedesktop/PackageKit", info.interfaces[0], call, prop, None)


Gio.bus_own_name(Gio.BusType.SESSION, "org.freedesktop.PackageKit", Gio.BusNameOwnerFlags.NONE, acquired, None, None)
GLib.MainLoop().run()
