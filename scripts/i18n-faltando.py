#!/usr/bin/env python3
"""Catalogs of crates/kalango-i18n/locales: what is missing from each one, compared with pt_BR.json.

pt_BR.json is the reference: every new UI string (`tr("…")`) gets its pt_BR entry first, and every other catalog
must have the same keys (the kalango-i18n tests fail otherwise). This script helps keep them in step:

  scripts/i18n-faltando.py                     # missing and extra keys per catalog (exit 1 when any)
  scripts/i18n-faltando.py --exportar DIR      # DIR/<locale>.json with the missing keys (English → English) to translate
  scripts/i18n-faltando.py --aplicar de DIR/de.json   # adds the translated keys to de.json, in pt_BR's order
  scripts/i18n-faltando.py --ordenar           # rewrites every catalog in pt_BR's order and layout (drops extras)

The layout is pt_BR's: one entry per line, one space of indentation, UTF-8 without escapes, a newline at the end.
"""
import argparse
import json
import sys
from pathlib import Path

LOCALES = Path(__file__).resolve().parent.parent / "crates" / "kalango-i18n" / "locales"


def load(name):
    return json.loads((LOCALES / f"{name}.json").read_text(encoding="utf-8"))


def write(name, entries):
    """Writes a catalog in pt_BR's layout (json.dumps with indent=1, ensure_ascii=False)."""
    text = json.dumps(entries, ensure_ascii=False, indent=1) + "\n"
    (LOCALES / f"{name}.json").write_text(text, encoding="utf-8")


def others():
    return sorted(p.stem for p in LOCALES.glob("*.json") if p.stem != "pt_BR")


def in_reference_order(reference, catalog):
    return {k: catalog[k] for k in reference if k in catalog}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--exportar", metavar="DIR")
    ap.add_argument("--aplicar", nargs=2, metavar=("LOCALE", "FILE"))
    ap.add_argument("--ordenar", action="store_true")
    a = ap.parse_args()
    ref = load("pt_BR")

    if a.aplicar:
        name, path = a.aplicar
        cat = load(name)
        new = json.loads(Path(path).read_text(encoding="utf-8"))
        unknown = [k for k in new if k not in ref]
        if unknown:
            sys.exit(f"keys not in pt_BR.json: {unknown}")
        empty = [k for k, v in new.items() if not str(v).strip()]
        if empty:
            sys.exit(f"empty translations: {empty}")
        cat.update(new)
        write(name, in_reference_order(ref, cat))
        print(f"{name}.json: {len(new)} entries applied, {len(ref) - len(set(cat) & set(ref))} still missing")
        return

    if a.ordenar:
        for name in others():
            cat = load(name)
            extra = [k for k in cat if k not in ref]
            write(name, in_reference_order(ref, cat))
            print(f"{name}.json: in pt_BR's order" + (f", {len(extra)} keys not in pt_BR.json dropped" if extra else ""))
        return

    bad = False
    for name in others():
        cat = load(name)
        missing = [k for k in ref if k not in cat]
        extra = [k for k in cat if k not in ref]
        if a.exportar:
            out = Path(a.exportar)
            out.mkdir(parents=True, exist_ok=True)
            (out / f"{name}.json").write_text(json.dumps({k: k for k in missing}, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
        print(f"{name}.json: {len(cat)} keys, {len(missing)} missing, {len(extra)} extra")
        for k in missing:
            print(f"  missing: {k}")
        for k in extra:
            print(f"  extra:   {k}")
        bad |= bool(missing or extra)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
