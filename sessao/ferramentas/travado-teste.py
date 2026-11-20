#!/usr/bin/python3
"""KalangoOS · an app that stops responding, for tests: a GTK 4 window ("Planilha", org.exemplo.Planilha) whose
interface thread, a moment after it shows, waits for a child process ("planilha-calculo", which holds ~200 MB) — as an
app stuck on a heavy tab or a calculation. While it waits, it reads none of the compositor's events. Ending the child
(the least destructive way out) gives it back: the window answers again. usage: travado-teste.py [log file]"""
import os
import subprocess
import sys
import gi
gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

LOG = sys.argv[1] if len(sys.argv) > 1 else None


def log(s):
    if LOG:
        with open(LOG, "a") as f:
            f.write(s + "\n")


def stall():
    child = subprocess.Popen(["planilha-calculo", "-c", "import time; x = bytearray(200 * 1024 * 1024); x[::4096] = b'1' * len(x[::4096]); time.sleep(600)"],
                             executable=sys.executable)
    log(f"travado: esperando o filho {child.pid}")
    child.wait()
    log("travado: respondendo de novo")
    label.set_text("Calculado.")
    return False


app = Gtk.Application(application_id="org.exemplo.Planilha")
label = None


def activate(a):
    global label
    w = Gtk.ApplicationWindow(application=a, title="Planilha — vendas.ods")
    w.set_default_size(700, 420)
    label = Gtk.Label(label="Calculando a planilha…")
    w.set_child(label)
    w.present()
    log(f"travado: janela aberta (pid {os.getpid()})")
    GLib.timeout_add(1500, stall)


app.connect("activate", activate)
app.run([])
