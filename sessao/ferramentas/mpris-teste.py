#!/usr/bin/python3
"""A stand-in media player on the session bus (MPRIS), for tests and captures only: org.mpris.MediaPlayer2.teste
playing "<title>" by "<artist>". usage: mpris-teste.py [title] [artist] [log file]

PlayPause, Next and Previous are written to the log file (so a test can check that the shell sent them) and
PlayPause toggles PlaybackStatus (with PropertiesChanged, as real players do)."""
import sys
from gi.repository import Gio, GLib

TITLE = sys.argv[1] if len(sys.argv) > 1 else "Alright"
ARTIST = sys.argv[2] if len(sys.argv) > 2 else "Jonathan Ogden"
LOG = sys.argv[3] if len(sys.argv) > 3 else None
PATH = "/org/mpris/MediaPlayer2"
state = {"status": "Playing"}

XML = """<node>
<interface name="org.mpris.MediaPlayer2">
  <method name="Raise"/><method name="Quit"/>
  <property name="Identity" type="s" access="read"/>
  <property name="DesktopEntry" type="s" access="read"/>
  <property name="CanQuit" type="b" access="read"/><property name="CanRaise" type="b" access="read"/>
  <property name="HasTrackList" type="b" access="read"/>
  <property name="SupportedUriSchemes" type="as" access="read"/><property name="SupportedMimeTypes" type="as" access="read"/>
</interface>
<interface name="org.mpris.MediaPlayer2.Player">
  <method name="PlayPause"/><method name="Play"/><method name="Pause"/><method name="Stop"/>
  <method name="Next"/><method name="Previous"/>
  <property name="PlaybackStatus" type="s" access="read"/>
  <property name="Metadata" type="a{sv}" access="read"/>
  <property name="CanPlay" type="b" access="read"/><property name="CanPause" type="b" access="read"/>
  <property name="CanGoNext" type="b" access="read"/><property name="CanGoPrevious" type="b" access="read"/>
  <property name="CanControl" type="b" access="read"/>
</interface>
</node>"""


def metadata():
    return {
        "mpris:trackid": GLib.Variant("o", "/org/mpris/MediaPlayer2/faixa/1"),
        "xesam:title": GLib.Variant("s", TITLE),
        "xesam:artist": GLib.Variant("as", [ARTIST]),
        "mpris:length": GLib.Variant("x", 214_000_000),
    }


def get(conn, sender, path, iface, prop):
    if iface == "org.mpris.MediaPlayer2":
        return {"Identity": GLib.Variant("s", "Música"), "DesktopEntry": GLib.Variant("s", "teste"),
                "SupportedUriSchemes": GLib.Variant("as", []), "SupportedMimeTypes": GLib.Variant("as", [])}.get(prop, GLib.Variant("b", False))
    if prop == "PlaybackStatus":
        return GLib.Variant("s", state["status"])
    if prop == "Metadata":
        return GLib.Variant("a{sv}", metadata())
    return GLib.Variant("b", True)


def call(conn, sender, path, iface, method, params, inv):
    if LOG:
        with open(LOG, "a") as f:
            f.write(method + "\n")
    if method in ("PlayPause", "Play", "Pause"):
        state["status"] = {"PlayPause": "Paused" if state["status"] == "Playing" else "Playing", "Play": "Playing", "Pause": "Paused"}[method]
        conn.emit_signal(None, PATH, "org.freedesktop.DBus.Properties", "PropertiesChanged",
                         GLib.Variant("(sa{sv}as)", ("org.mpris.MediaPlayer2.Player", {"PlaybackStatus": GLib.Variant("s", state["status"])}, [])))
    inv.return_value(None)


def acquired(conn, name):
    node = Gio.DBusNodeInfo.new_for_xml(XML)
    for i in node.interfaces:
        conn.register_object(PATH, i, call, get, None)


Gio.bus_own_name(Gio.BusType.SESSION, "org.mpris.MediaPlayer2.teste", Gio.BusNameOwnerFlags.NONE, acquired, None, None)
GLib.MainLoop().run()
