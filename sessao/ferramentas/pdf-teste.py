#!/usr/bin/env python3
"""A small PDF with real text (three pages, Portuguese accents) for the document reader's session test.
No library needed: the standard Helvetica font, WinAnsi encoding, the cross-reference table counted by hand."""
import sys

PAGES = [
    [("Relatório de outubro", 24), ("Resultados e pendências do mês", 13), ("", 12),
     ("Este relatório resume o mês de outubro da equipe de operações.", 12),
     ("As contas, os contratos e as pendências estão nas próximas páginas.", 12)],
    [("Receitas e despesas", 18), ("", 12),
     ("A receita subiu 8% sobre setembro, puxada pelos contratos novos.", 12),
     ("As despesas ficaram estáveis em relação ao mês anterior.", 12),
     ("O caixa terminou o mês com folga para o investimento de novembro.", 12)],
    [("Pendências", 18), ("", 12),
     ("Há três pendências: a fatura em duplicidade, o contrato a renovar e o backup.", 12),
     ("A fatura em duplicidade deve ser estornada até sexta-feira.", 12),
     ("A próxima revisão está marcada para 20 de novembro.", 12)],
]


def esc(t: str) -> bytes:
    return t.encode("cp1252").replace(b"\\", b"\\\\").replace(b"(", b"\\(").replace(b")", b"\\)")


def main(out: str) -> None:
    objs: list[bytes] = []
    n = len(PAGES)
    # 1 catalog, 2 pages, 3 font; then each page and its content
    kids = " ".join(f"{4 + 2 * i} 0 R" for i in range(n))
    objs.append(b"<< /Type /Catalog /Pages 2 0 R >>")
    objs.append(f"<< /Type /Pages /Kids [{kids}] /Count {n} >>".encode())
    objs.append(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    for i, lines in enumerate(PAGES):
        y = 770
        body = b""
        for text, size in lines:
            if text:
                body += b"BT /F1 %d Tf 72 %d Td (" % (size, y) + esc(text) + b") Tj ET\n"
            y -= int(size * 1.9)
        objs.append(f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 3 0 R >> >> /Contents {5 + 2 * i} 0 R >>".encode())
        objs.append(b"<< /Length %d >>\nstream\n" % len(body) + body + b"endstream")
    pdf = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n"
    offsets = []
    for k, o in enumerate(objs, 1):
        offsets.append(len(pdf))
        pdf += b"%d 0 obj\n" % k + o + b"\nendobj\n"
    xref = len(pdf)
    pdf += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objs) + 1)
    for off in offsets:
        pdf += b"%010d 00000 n \n" % off
    pdf += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objs) + 1, xref)
    with open(out, "wb") as f:
        f.write(pdf)


if __name__ == "__main__":
    main(sys.argv[1])
