"""Converts the one-page PDFs of generate.mjs to SVG (text as glyph paths: no font needed to view).

usage: python3 to-svg.py <pdf>...   (needs PyMuPDF: pip install pymupdf)
"""

import re
import sys

import pymupdf

for pdf in sys.argv[1:]:
    with pymupdf.open(pdf) as doc:
        svg = doc[0].get_svg_image(text_as_path=True)
    # Pixel size instead of points: the PDF page is the CSS viewport at 96 dpi.
    def px(m):
        return f'{m.group(1)}="{round(float(m.group(2)) * 96 / 72)}"'

    head, rest = svg.split(">", 1)
    head = re.sub(r'\b(width|height)="([\d.]+)(?:pt)?"', px, head)
    out = pdf[: -len(".pdf")] + ".svg"
    with open(out, "w", encoding="utf-8") as f:
        f.write(head + ">" + rest)
    print(out)
