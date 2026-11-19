#!/usr/bin/env python3
"""KalangoOS · small fixed-size boxes (an icon's tile, a dot, a badge) that the row they are in would stretch.

GTK gives a child the whole height of a horizontal row (and the whole width of a column) unless it is aligned: a
22 × 22 tile without an alignment becomes a tall, squeezed pill. Prints each one and exits 1 when there is any.
Run from the repository root.
"""
import glob, re, sys

ruins = []
for f in sorted(glob.glob("crates/kalango-shell/src/**/*.rs", recursive=True)):
    linhas = open(f).read().splitlines()
    for i, l in enumerate(linhas):
        m = re.search(r'let (?:mut )?(\w+) = caixa\(Orientation::\w+, \d+, "([\w-]*)"\);', l)
        if not m:
            continue
        var, classe = m.groups()
        bloco = "\n".join(linhas[i + 1:i + 8])
        s = re.search(re.escape(var) + r"\.set_size_request\((\d+), (\d+)\)", bloco)
        if not s:
            continue
        w, h = int(s.group(1)), int(s.group(2))
        if not (2 <= w <= 40 and 2 <= h <= 40):
            continue
        if re.search(re.escape(var) + r"\.set_(valign|halign)\(", bloco):
            continue
        ruins.append(f"{f}:{i + 1}  {var} \"{classe}\" {w}×{h} sem alinhamento")
for r in ruins:
    print(r)
print(f"{len(ruins)} caixas pequenas sem alinhamento", file=sys.stderr)
sys.exit(1 if ruins else 0)
