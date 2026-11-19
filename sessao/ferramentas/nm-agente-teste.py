#!/usr/bin/python3
"""KalangoOS · NetworkManager de teste para o agente de senhas, no barramento da *sessão* (o shell o usa quando
KALANGO_NM_BUS=session): tem o AgentManager (Register, RegisterWithCapabilities, Unregister) e, quando aparece
<dir>/pedir.json ({"ssid": …, "key_mgmt": "sae", "flags": 1}), pede GetSecrets ao agente registrado como o
NetworkManager pediria; a resposta (os segredos, ou o nome do erro) vai para <dir>/resposta.json e cada passo para
<dir>/nm.log. usage: nm-agente-teste.py <dir>"""
import json
import os
import sys

from gi.repository import Gio, GLib

DIR = sys.argv[1] if len(sys.argv) > 1 else "/tmp"
PEDIR = os.path.join(DIR, "pedir.json")
RESPOSTA = os.path.join(DIR, "resposta.json")
LOG = os.path.join(DIR, "nm.log")

XML = """<node><interface name="org.freedesktop.NetworkManager.AgentManager">
  <method name="Register"><arg name="identifier" type="s" direction="in"/></method>
  <method name="RegisterWithCapabilities"><arg name="identifier" type="s" direction="in"/><arg name="capabilities" type="u" direction="in"/></method>
  <method name="Unregister"/>
</interface></node>"""

agente = {"nome": None}
conexao = {"c": None}


def anotar(t):
    with open(LOG, "a") as f:
        f.write(t + "\n")


def chamada(conn, sender, path, iface, method, params, inv):
    if method in ("Register", "RegisterWithCapabilities"):
        agente["nome"] = sender
        anotar("registrado %s" % params.unpack()[0])
    elif method == "Unregister":
        agente["nome"] = None
        anotar("saiu")
    inv.return_value(None)


def resposta(conn, res):
    try:
        v = conn.call_finish(res)
        segredos = v.unpack()[0]
        json.dump({"segredos": segredos}, open(RESPOSTA, "w"), ensure_ascii=False)
        anotar("segredos recebidos")
    except GLib.Error as e:
        nome = Gio.DBusError.get_remote_error(e) or str(e)
        json.dump({"erro": nome}, open(RESPOSTA, "w"))
        anotar("erro %s" % nome)


def olhar():
    if os.path.exists(PEDIR) and agente["nome"]:
        p = json.load(open(PEDIR))
        os.remove(PEDIR)
        ssid = p.get("ssid", "Biblioteca")
        con = {
            "connection": {"id": GLib.Variant("s", ssid), "uuid": GLib.Variant("s", "5d1c0b3e-6c1b-4f0e-9a43-6f1b2c3d4e5f"),
                           "type": GLib.Variant("s", "802-11-wireless")},
            "802-11-wireless": {"ssid": GLib.Variant("ay", ssid.encode())},
            "802-11-wireless-security": {"key-mgmt": GLib.Variant("s", p.get("key_mgmt", "sae"))},
        }
        anotar("pedindo %s" % ssid)
        conexao["c"].call(agente["nome"], "/org/freedesktop/NetworkManager/SecretAgent", "org.freedesktop.NetworkManager.SecretAgent",
                          "GetSecrets", GLib.Variant("(a{sa{sv}}osasu)", (con, "/org/freedesktop/NetworkManager/Settings/9",
                                                                         p.get("setting", "802-11-wireless-security"), [], p.get("flags", 1))),
                          GLib.VariantType("(a{sa{sv}})"), Gio.DBusCallFlags.NONE, 120000, None, resposta)
    return True


def adquirido(conn, nome):
    conexao["c"] = conn
    info = Gio.DBusNodeInfo.new_for_xml(XML)
    conn.register_object("/org/freedesktop/NetworkManager/AgentManager", info.interfaces[0], chamada, None, None)
    anotar("pronto")


Gio.bus_own_name(Gio.BusType.SESSION, "org.freedesktop.NetworkManager", Gio.BusNameOwnerFlags.NONE, adquirido, None, None)
GLib.timeout_add(300, olhar)
GLib.MainLoop().run()
