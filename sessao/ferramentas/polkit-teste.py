#!/usr/bin/python3
"""KalangoOS · polkitd de teste para o agente de autenticação, no barramento da *sessão* (o shell o usa quando
KALANGO_POLKIT_BUS=session): a Authority aceita RegisterAuthenticationAgent e, quando aparece <dir>/pedir.json
({"acao": …, "mensagem": …, "comando": …, "manter": false}), pede BeginAuthentication ao agente como o polkitd pediria;
o resultado (ok, ou o nome do erro) vai para <dir>/resposta.json e cada passo para <dir>/polkit.log.
usage: polkit-teste.py <dir>"""
import json
import os
import sys

from gi.repository import Gio, GLib

DIR = sys.argv[1] if len(sys.argv) > 1 else "/tmp"
PEDIR, RESPOSTA, LOG = (os.path.join(DIR, n) for n in ("pedir.json", "resposta.json", "polkit.log"))
XML = """<node><interface name="org.freedesktop.PolicyKit1.Authority">
  <method name="RegisterAuthenticationAgent"><arg type="(sa{sv})" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/></method>
  <method name="UnregisterAuthenticationAgent"><arg type="(sa{sv})" direction="in"/><arg type="s" direction="in"/></method>
</interface></node>"""
estado = {"agente": None, "caminho": None, "conn": None}


def anotar(t):
    with open(LOG, "a") as f:
        f.write(t + "\n")


def chamada(conn, sender, path, iface, method, params, inv):
    if method == "RegisterAuthenticationAgent":
        sujeito, local, caminho = params.unpack()
        estado["agente"], estado["caminho"] = sender, caminho
        anotar("registrado %s %s" % (sujeito[0], caminho))
    inv.return_value(None)


def resposta(conn, res):
    try:
        conn.call_finish(res)
        json.dump({"ok": True}, open(RESPOSTA, "w"))
        anotar("autorizado")
    except GLib.Error as e:
        nome = Gio.DBusError.get_remote_error(e) or str(e)
        json.dump({"erro": nome}, open(RESPOSTA, "w"))
        anotar("erro %s" % nome)


def olhar():
    if os.path.exists(PEDIR) and estado["agente"]:
        p = json.load(open(PEDIR))
        os.remove(PEDIR)
        detalhes = {"polkit.caller-pid": str(os.getpid()), "polkit.subject-pid": str(os.getpid())}
        if p.get("comando"):
            detalhes["command_line"] = p["comando"]
            detalhes["program"] = p["comando"].split()[0]
        if p.get("manter"):
            detalhes["polkit.retains_authorization_after_challenge"] = "1"
        ids = [("unix-user", {"uid": GLib.Variant("u", os.getuid())})]
        anotar("pedindo %s" % p.get("acao"))
        estado["conn"].call(estado["agente"], estado["caminho"], "org.freedesktop.PolicyKit1.AuthenticationAgent", "BeginAuthentication",
                            GLib.Variant("(sssa{ss}sa(sa{sv}))", (p.get("acao", "org.kalango.teste"), p.get("mensagem", "Autenticação necessária"), "",
                                                                  detalhes, "biscoito-teste", ids)),
                            None, Gio.DBusCallFlags.NONE, 120000, None, resposta)
    return True


def adquirido(conn, nome):
    estado["conn"] = conn
    conn.register_object("/org/freedesktop/PolicyKit1/Authority", Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0], chamada, None, None)
    anotar("pronto")


Gio.bus_own_name(Gio.BusType.SESSION, "org.freedesktop.PolicyKit1", Gio.BusNameOwnerFlags.NONE, adquirido, None, None)
GLib.timeout_add(300, olhar)
GLib.MainLoop().run()
