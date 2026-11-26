#!/usr/bin/env python3
"""Writes the installable hicolor theme from the application icons. Run from this folder:
python3 export.py

`apps/<id>.svg` are the author's icons, on a 24 grid; `apps/<id>-16.svg` are the optical
versions for 16 px, drawn on a 16 grid with every line on whole pixels. This script copies the
SVGs to `hicolor/scalable/apps/` and renders each optical version to
`hicolor/16x16/apps/<id>.png`. The other PNGs of `16x16` and `32x32` are the author's, copied as
they are. It also writes `legibility-16.png`: each icon at 16 px as drawn, then its optical
version, enlarged eight times without smoothing. The renders go through librsvg (Debian
packages gir1.2-rsvg-2.0 and python3-gi), like the logo.
"""
import os
import shutil

import gi

gi.require_version("Rsvg", "2.0")
gi.require_version("GdkPixbuf", "2.0")
from gi.repository import GdkPixbuf, Rsvg  # noqa: E402

IDS = ("evidence", "inquiry", "control-room", "sherlock", "lab", "games", "installer")
OPTICAL = "-16"
# Night blue of the desktop, behind the legibility board.
DESKTOP = 0x0E1A2EFF


def image(path, size):
    with open(path) as f:
        source = f.read()
    # The width and height of the root element set the size of the render.
    head, rest = source.split(">", 1)
    for key in ("width", "height"):
        start = head.index(f'{key}="') + len(key) + 2
        end = head.index('"', start)
        head = f"{head[:start]}{size}{head[end:]}"
    return Rsvg.Handle.new_from_data(f"{head}>{rest}".encode()).get_pixbuf()


def enlarged(rows, factor=8, gap=4):
    """The images of each row side by side, enlarged without smoothing."""
    size = rows[0][0].get_width() * factor
    width = gap + len(rows[0]) * (size + gap)
    height = gap + len(rows) * (size + gap)
    board = GdkPixbuf.Pixbuf.new(GdkPixbuf.Colorspace.RGB, True, 8, width, height)
    board.fill(DESKTOP)
    for j, row in enumerate(rows):
        for i, img in enumerate(row):
            x, y = gap + i * (size + gap), gap + j * (size + gap)
            img.composite(board, x, y, size, size, x, y, factor, factor,
                          GdkPixbuf.InterpType.NEAREST, 255)
    return board


optical = [i for i in IDS if os.path.exists(f"apps/{i}{OPTICAL}.svg")]
for icon in IDS:
    shutil.copyfile(f"apps/{icon}.svg", f"hicolor/scalable/apps/{icon}.svg")
for icon in optical:
    image(f"apps/{icon}{OPTICAL}.svg", 16).savev(f"hicolor/16x16/apps/{icon}.png", "png", [], [])
before = [image(f"apps/{icon}.svg", 16) for icon in optical]
after = [image(f"apps/{icon}{OPTICAL}.svg", 16) for icon in optical]
enlarged([before + after]).savev("legibility-16.png", "png", [], [])
