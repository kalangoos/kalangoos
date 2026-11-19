#!/usr/bin/python3
"""KalangoOS · BlueZ de teste para o agente de pareamento, no barramento da *sessão* (o shell o usa quando
KALANGO_BLUEZ_BUS=session). Os aparelhos são os do bluetoothctl de teste (o mesmo estado, KALANGO_BLUETOOTHCTL_ESTADO)
e mais dois que só pedem pareamento: "Teclado K3" (digita um código) e "Pixel 8" (compara um código).

- AgentManager1 em /org/bluez (RegisterAgent, RequestDefaultAgent, UnregisterAgent);
- ObjectManager em / e um Device1 por aparelho (Address, Alias, Icon, Paired; Pair e CancelPairing);
- Pair: um teclado recebe DisplayPasskey(482913) com os dígitos chegando de meio em meio segundo; um celular,
  RequestConfirmation(482913); os outros, sem perguntas. Pareado: o estado muda, sai PropertiesChanged(Paired).
- <dir>/parear.json ({"mac": …}) faz o aparelho pedir o pareamento sozinho (como um teclado ligado perto);
- <dir>/conectar.json ({"mac": …, "conectado": true, "bateria": 80}) liga ou desliga um aparelho: muda Connected
  (PropertiesChanged) e a bateria (Battery1.Percentage);
  <dir>/bluez.log anota cada passo. usage: bluez-teste.py <dir>"""
import json
import os
import sys

from gi.repository import Gio, GLib

DIR = sys.argv[1] if len(sys.argv) > 1 else "/tmp"
ESTADO = os.environ.get("KALANGO_BLUETOOTHCTL_ESTADO", os.path.join(DIR, "bluetoothctl.json"))
PEDIR = os.path.join(DIR, "parear.json")
CONECTAR = os.path.join(DIR, "conectar.json")
LOG = os.path.join(DIR, "bluez.log")
CODIGO = 482913
EXTRAS = [
    {"mac": "AA:BB:CC:00:11:22", "nome": "Teclado K3", "icone": "input-keyboard", "pareado": False, "conectado": False},
    {"mac": "AA:BB:CC:33:44:55", "nome": "Pixel 8", "icone": "phone", "pareado": False, "conectado": False},
]

XML_MGR = """<node><interface name="org.bluez.AgentManager1">
  <method name="RegisterAgent"><arg type="o" direction="in"/><arg type="s" direction="in"/></method>
  <method name="RequestDefaultAgent"><arg type="o" direction="in"/></method>
  <method name="UnregisterAgent"><arg type="o" direction="in"/></method>
</interface></node>"""
XML_OM = """<node><interface name="org.freedesktop.DBus.ObjectManager">
  <method name="GetManagedObjects"><arg type="a{oa{sa{sv}}}" direction="out"/></method>
</interface></node>"""
XML_DEV = """<node><interface name="org.bluez.Device1">
  <method name="Pair"/>
  <method name="CancelPairing"/>
  <property name="Address" type="s" access="read"/>
  <property name="Alias" type="s" access="read"/>
  <property name="Icon" type="s" access="read"/>
  <property name="Paired" type="b" access="read"/>
  <property name="Connected" type="b" access="read"/>
</interface></node>"""
XML_BAT = """<node><interface name="org.bluez.Battery1">
  <property name="Percentage" type="y" access="read"/>
</interface></node>"""

estado = {"agente": None, "dono": None, "conn": None, "parando": set(), "conectados": {}}


def anotar(t):
    with open(LOG, "a") as f:
        f.write(t + "\n")


def ler():
    try:
        e = json.load(open(ESTADO))
    except Exception:
        e = {"aparelhos": [], "perto": []}
    return e


def todos():
    e = ler()
    return e.get("aparelhos", []) + e.get("perto", []) + [x for x in EXTRAS if not any(y["mac"] == x["mac"] for y in e.get("aparelhos", []))]


def caminho(mac):
    return "/org/bluez/hci0/dev_" + mac.replace(":", "_")


def acha(path):
    return next((x for x in todos() if caminho(x["mac"]) == path), None)


def marcar_pareado(x):
    e = ler()
    if os.path.exists(ESTADO):
        achou = next((y for y in e.get("perto", []) if y["mac"] == x["mac"]), None)
        if achou:
            e["perto"].remove(achou)
            achou["pareado"] = True
            e["aparelhos"].append(achou)
        elif not any(y["mac"] == x["mac"] for y in e.get("aparelhos", [])):
            e.setdefault("aparelhos", []).append(dict(x, pareado=True))
        json.dump(e, open(ESTADO, "w"), ensure_ascii=False, indent=1)
    for y in EXTRAS:
        if y["mac"] == x["mac"]:
            y["pareado"] = True
    estado["conn"].emit_signal(None, caminho(x["mac"]), "org.freedesktop.DBus.Properties", "PropertiesChanged",
                               GLib.Variant("(sa{sv}as)", ("org.bluez.Device1", {"Paired": GLib.Variant("b", True)}, [])))
    anotar("pareado %s" % x["mac"])


def agente(metodo, args, fim):
    """Calls the agent; fim(ok, error name)."""
    def volta(conn, res):
        try:
            conn.call_finish(res)
            fim(True, None)
        except GLib.Error as e:
            fim(False, Gio.DBusError.get_remote_error(e) or str(e))
    estado["conn"].call(estado["dono"], estado["agente"], "org.bluez.Agent1", metodo, args, None, Gio.DBusCallFlags.NONE, 120000, None, volta)


def parear(x, fim):
    """The pairing conversation for one device; fim(ok, error)."""
    path = caminho(x["mac"])
    anotar("pareando %s (%s)" % (x["mac"], x["icone"]))
    if not estado["agente"]:
        fim(False, "org.bluez.Error.AuthenticationFailed")
        return
    if x["icone"].startswith("input-keyboard"):
        passo = {"n": 0}

        def proximo():
            if x["mac"] in estado["parando"]:
                estado["parando"].discard(x["mac"])
                anotar("cancelado %s" % x["mac"])
                agente("Cancel", None, lambda ok, e: None)
                fim(False, "org.bluez.Error.AuthenticationCanceled")
                return False
            agente("DisplayPasskey", GLib.Variant("(ouq)", (path, CODIGO, passo["n"])), lambda ok, e: None)
            anotar("digitou %d" % passo["n"])
            if passo["n"] >= 6:
                GLib.timeout_add(400, lambda: (marcar_pareado(x), fim(True, None), False)[2])
                return False
            passo["n"] += 1
            return True
        proximo()
        GLib.timeout_add(500, proximo)
    elif x["icone"].startswith("phone"):
        def respondeu(ok, erro):
            anotar("confirmação %s: %s" % (x["mac"], "sim" if ok else erro))
            if ok:
                marcar_pareado(x)
                fim(True, None)
            else:
                fim(False, "org.bluez.Error.AuthenticationRejected")
        agente("RequestConfirmation", GLib.Variant("(ou)", (path, CODIGO)), respondeu)
    else:
        marcar_pareado(x)
        fim(True, None)


def dispositivo(conn, sender, path, iface, method, params, inv):
    x = acha(path)
    if not x:
        inv.return_dbus_error("org.freedesktop.DBus.Error.UnknownObject", path)
        return
    if method == "Pair":
        anotar("Pair %s" % x["mac"])

        def fim(ok, erro):
            if ok:
                inv.return_value(None)
            else:
                inv.return_dbus_error(erro, "pairing failed")
        parear(x, fim)
    elif method == "CancelPairing":
        anotar("CancelPairing %s" % x["mac"])
        estado["parando"].add(x["mac"])
        inv.return_value(None)


def propriedade(conn, sender, path, iface, nome):
    x = acha(path)
    if not x:
        return None
    ligado, bateria = estado["conectados"].get(x["mac"], (bool(x.get("conectado")), x.get("bateria")))
    if iface == "org.bluez.Battery1":
        return GLib.Variant("y", int(bateria)) if nome == "Percentage" and bateria is not None else None
    return {"Address": GLib.Variant("s", x["mac"]), "Alias": GLib.Variant("s", x["nome"]), "Icon": GLib.Variant("s", x["icone"]),
            "Paired": GLib.Variant("b", bool(x["pareado"])), "Connected": GLib.Variant("b", ligado)}.get(nome)


def gerente(conn, sender, path, iface, method, params, inv):
    if method == "RegisterAgent":
        estado["agente"], estado["dono"] = params.unpack()[0], sender
        anotar("agente %s %s" % params.unpack())
    elif method == "RequestDefaultAgent":
        anotar("padrão %s" % params.unpack()[0])
    elif method == "UnregisterAgent":
        estado["agente"] = None
    inv.return_value(None)


def objetos(conn, sender, path, iface, method, params, inv):
    d = {}
    for x in todos():
        d[caminho(x["mac"])] = {"org.bluez.Device1": {"Address": GLib.Variant("s", x["mac"]), "Alias": GLib.Variant("s", x["nome"]),
                                                       "Icon": GLib.Variant("s", x["icone"]), "Paired": GLib.Variant("b", bool(x["pareado"]))}}
    inv.return_value(GLib.Variant("(a{oa{sa{sv}}})", (d,)))


registrados = set()


def registrar_aparelhos():
    info = Gio.DBusNodeInfo.new_for_xml(XML_DEV).interfaces[0]
    bat = Gio.DBusNodeInfo.new_for_xml(XML_BAT).interfaces[0]
    for x in todos():
        p = caminho(x["mac"])
        if p not in registrados:
            estado["conn"].register_object(p, info, dispositivo, propriedade, None)
            estado["conn"].register_object(p, bat, None, propriedade, None)
            registrados.add(p)


def olhar():
    registrar_aparelhos()
    if os.path.exists(CONECTAR):
        p = json.load(open(CONECTAR))
        os.remove(CONECTAR)
        x = next((y for y in todos() if y["mac"] == p.get("mac")), None)
        if x:
            ligado = bool(p.get("conectado", True))
            estado["conectados"][x["mac"]] = (ligado, p.get("bateria"))
            estado["conn"].emit_signal(None, caminho(x["mac"]), "org.freedesktop.DBus.Properties", "PropertiesChanged",
                                       GLib.Variant("(sa{sv}as)", ("org.bluez.Device1", {"Connected": GLib.Variant("b", ligado)}, [])))
            anotar("%s %s" % ("conectado" if ligado else "desconectado", x["mac"]))
    if os.path.exists(PEDIR) and estado["agente"]:
        p = json.load(open(PEDIR))
        os.remove(PEDIR)
        x = next((y for y in todos() if y["mac"] == p.get("mac")), None)
        if x:
            parear(x, lambda ok, e: anotar("pedido do aparelho %s: %s" % (x["mac"], "pareado" if ok else e)))
    return True


def adquirido(conn, nome):
    estado["conn"] = conn
    conn.register_object("/org/bluez", Gio.DBusNodeInfo.new_for_xml(XML_MGR).interfaces[0], gerente, None, None)
    conn.register_object("/", Gio.DBusNodeInfo.new_for_xml(XML_OM).interfaces[0], objetos, None, None)
    registrar_aparelhos()
    anotar("pronto")


Gio.bus_own_name(Gio.BusType.SESSION, "org.bluez", Gio.BusNameOwnerFlags.NONE, adquirido, None, None)
GLib.timeout_add(300, olhar)
GLib.MainLoop().run()
