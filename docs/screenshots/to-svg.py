"""Converts the one-page PDFs of generate.mjs to SVG (text as glyph paths: no font needed to view).

usage: python3 to-svg.py <pdf>...   (needs PyMuPDF: pip install pymupdf)

PyMuPDF writes each glyph once (`<path id="font_…">`, em units) and places every character with
`<use xlink:href transform="matrix(size,0,0,-size,x,y)" fill>`; the output is made smaller by
rounding the numbers, shortening the glyph ids and grouping the runs of characters of the same
size and color in a `<g>` (the characters then only carry `x` / `y`, in em units).
"""

import re
import sys

import pymupdf

NUMBER = re.compile(r"-?\d*\.\d+|-?\d+")
USE = re.compile(
    r'<use(?: data-text="[^"]*")? xlink:href="#([\w-]+)" '
    r'transform="matrix\(([-\d.]+),0,0,([-\d.]+),([-\d.]+),([-\d.]+)\)"([^/]*)/>\n'
)


def num(value, decimals):
    """`0.24511719` -> `.245` (3 decimals)."""
    s = f"{float(value):.{decimals}f}".rstrip("0").rstrip(".")
    if s in ("-0", ""):
        return "0"
    if s.startswith("0."):
        return s[1:]
    if s.startswith("-0."):
        return "-" + s[2:]
    return s


def shorten(text, decimals):
    def fix(m):
        s = num(m.group(0), decimals)
        # `.12-.0004` -> `.12 0`: the minus sign was the separator.
        return " " + s if m.group(0)[0] == "-" and s[0] != "-" else s

    return NUMBER.sub(fix, text)


def short_id(i):
    digits = "abcdefghijklmnopqrstuvwxyz0123456789"
    out = digits[i % 26]
    i //= 26
    while i:
        out += digits[i % 36]
        i //= 36
    return out


def group_uses(svg, ids):
    """Consecutive `<use>` with the same scale and attributes -> one `<g>`."""
    out, pos, run, key = [], 0, [], None

    def flush():
        if not run:
            return
        a, d, attrs = key
        out.append(f'<g transform="matrix({num(a, 3)},0,0,{num(d, 3)},0,0)"{attrs}>')
        for gid, x, y in run:
            # Translation in em units: divided by the scale.
            out.append(f'<use href="#{ids[gid]}" x="{num(float(x) / float(a), 3)}" y="{num(float(y) / float(d), 3)}"/>')
        out.append("</g>\n")
        run.clear()

    for m in USE.finditer(svg):
        if m.start() != pos:
            flush()
            out.append(svg[pos : m.start()])
        gid, a, d, x, y, attrs = m.groups()
        k = (a, d, attrs.rstrip())
        if k != key:
            flush()
            key = k
        run.append((gid, x, y))
        pos = m.end()
    flush()
    out.append(svg[pos:])
    return "".join(out)


def convert(pdf):
    with pymupdf.open(pdf) as doc:
        svg = doc[0].get_svg_image(text_as_path=True)
    head, rest = svg.split(">", 1)
    # Pixel size instead of points: the PDF page is the CSS viewport at 96 dpi.
    head = re.sub(
        r'\b(width|height)="([\d.]+)(?:pt)?"',
        lambda m: f'{m.group(1)}="{round(float(m.group(2)) * 96 / 72)}"',
        head,
    )
    head = head.replace(' xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape"', "")
    ids = {gid: short_id(i) for i, gid in enumerate(dict.fromkeys(re.findall(r'<path id="(font_[\w-]+)"', rest)))}
    rest = group_uses(rest, ids)
    # Glyph outlines in em units (3 decimals = 1/60 px at 16 px), the rest in points.
    rest = re.sub(r'<path id="(font_[\w-]+)" d="([^"]*)"', lambda m: f'<path id="{ids[m.group(1)]}" d="{shorten(m.group(2), 3)}"', rest)
    rest = re.sub(r'\b(d|transform)="([^"]*)"', lambda m: f'{m.group(1)}="{shorten(m.group(2), 3)}"', rest)
    out = pdf[: -len(".pdf")] + ".svg"
    with open(out, "w", encoding="utf-8") as f:
        f.write(head + ">" + rest)
    return out


for pdf in sys.argv[1:]:
    print(convert(pdf))
