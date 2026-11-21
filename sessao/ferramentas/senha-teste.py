#!/usr/bin/python3
"""A stand-in app window with a password field (GTK 4), for the screen-control test: an agent may click it, never type
into it (kalangod reads the field's AT-SPI role). usage: senha-teste.py"""
import gi

gi.require_version("Gtk", "4.0")
from gi.repository import Gtk  # noqa: E402


def ativar(app):
    w = Gtk.ApplicationWindow(application=app, title="Entrar no servico de teste")
    w.set_default_size(420, 220)
    caixa = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12, margin_top=24, margin_bottom=24, margin_start=24, margin_end=24)
    caixa.append(Gtk.Label(label="Usuario"))
    caixa.append(Gtk.Entry(text="pessoa-de-teste"))
    caixa.append(Gtk.Label(label="Senha"))
    senha = Gtk.PasswordEntry()
    caixa.append(senha)
    w.set_child(caixa)
    w.present()
    senha.grab_focus()


app = Gtk.Application(application_id="os.kalango.TesteSenha")
app.connect("activate", ativar)
app.run(None)
