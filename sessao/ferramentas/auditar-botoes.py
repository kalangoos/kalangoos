#!/usr/bin/env python3
"""KalangoOS · text buttons whose style leaves the text against their edges.

The shell's sheet starts with `button { all: unset; }`: a button with text has no padding until its own rule gives one.
A text button with a background or a border needs at least 8 px on each side (design: 10–16 px). Prints the offenders
and exits 1 when there is any. Run from the repository root.
"""
import glob, re, sys

CSS = "crates/kalango-shell/assets/style.css"
SRC = "crates/kalango-shell/src/**/*.rs"
regras = [(s.strip(), b) for s, b in re.findall(r"([^{}]+)\{([^{}]*)\}", open(CSS).read())]

def props(corpo):
    d = {}
    for parte in corpo.split(";"):
        if ":" in parte:
            k, v = parte.split(":", 1)
            d[k.strip()] = v.strip()
    return d

def lados(p):
    px = [float(x[:-2]) if x.endswith("px") else 0.0 for x in p.split()]
    if not px:
        return 0.0, 0.0
    return (px[0], px[0]) if len(px) == 1 else (px[1], px[1]) if len(px) in (2, 3) else (px[1], px[3])

def aplica(seletor, classes):
    for parte in seletor.split(","):
        parte = parte.strip()
        if not parte or ":" in parte.split()[-1]:
            continue
        ultimo = parte.split()[-1].split(">")[-1]
        cs = re.findall(r"\.([\w-]+)", ultimo)
        if cs and all(c in classes for c in cs):
            return True
    return False

ruins = []
for f in sorted(glob.glob(SRC, recursive=True)):
    linhas = open(f).read().splitlines()
    for i, l in enumerate(linhas):
        m = re.search(r"let (?:mut )?(\w+) = gtk::(Button|ToggleButton|MenuButton)::(with_label\(|new\(\))", l)
        if not m:
            continue
        var, bloco = m.group(1), linhas[i + 1:i + 16]
        texto = m.group(3) == "with_label("
        classes = set()
        for b in bloco:
            classes |= set(re.findall(re.escape(var) + r'\.add_css_class\("([\w-]+)"\)', b))
            if re.search(re.escape(var) + r"\.(set_child\(Some\(&rotulo\(|set_label\()", b):
                texto = True
        if not texto or not classes:
            continue
        padding, fundo = None, False
        for s, corpo in regras:
            if aplica(s, classes):
                p = props(corpo)
                padding = p.get("padding", padding)
                bg = p.get("background", p.get("background-color", ""))
                fundo = fundo or (bg not in ("", "none", "transparent")) or p.get("border", "none") not in ("none", "0")
        esq, dir_ = lados(padding) if padding else (0.0, 0.0)
        if fundo and min(esq, dir_) < 8:
            ruins.append(f"{f}:{i + 1}  .{' .'.join(sorted(classes))}  padding: {padding or 'nenhum'}")
for r in ruins:
    print(r)
print(f"{len(ruins)} botões de texto sem espaço nas laterais", file=sys.stderr)
sys.exit(1 if ruins else 0)
