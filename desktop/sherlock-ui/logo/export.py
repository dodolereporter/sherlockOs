#!/usr/bin/env python3
"""Writes the logo SVGs and their PNGs. Run from this folder: python3 export.py

The symbol is drawn on a 512 grid: ring centered at (256, 256), a tail as an arc that leaves
the ring and ends at the dot, background square of 512 with radius 96. The optical versions
for 32 and 16 px thicken the ring, enlarge the symbol within the square and shorten the tail,
so that the drawing holds on so few pixels. The PNGs go through librsvg (Debian packages
gir1.2-rsvg-2.0 and python3-gi).
"""
import gi

gi.require_version("Rsvg", "2.0")
gi.require_version("GdkPixbuf", "2.0")
from gi.repository import GdkPixbuf, Rsvg  # noqa: E402

NIGHT, GREEN, CREAM = "#0E1A2E", "#85C595", "#F4EBE3"
# Name: (square background, ring and tail, dot)
VARIANTS = {
    "logo": (NIGHT, GREEN, CREAM),
    "logo-cream": (NIGHT, CREAM, CREAM),
    "logo-night": (CREAM, NIGHT, NIGHT),
}
# Geometries, in units of the 512 grid: ring radius and width, start of the tail on the ring
# (3-4-5 triangle: 53° below the horizontal), end point, tail radius and width, dot radius.
GEOMETRIES = {
    "": dict(r=140, width=36, start=(340, 368), dot=(308, 256), arc=80, tail=28, rd=22),
    # 32 px: 1 px is 16 units. Ring of 3 px, enlarged symbol.
    "-32": dict(r=175, width=50, start=(361, 396), dot=(300, 248), arc=100, tail=36, rd=36),
    # 16 px: 1 px is 32 units. Ring of 1.75 px, margin of 1.5 px, short and thick tail.
    "-16": dict(r=180, width=56, start=(364, 400), dot=(300, 252), arc=110, tail=40, rd=38),
}
SIZES = (16, 32, 64, 256, 512)


def svg(background, stroke, dot, square, g):
    box = "0 0 512 512" if square else "96 96 320 320"
    rect = f'\n  <rect width="512" height="512" rx="96" fill="{background}"/>' if square else ""
    (sx, sy), (dx, dy) = g["start"], g["dot"]
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="{box}">{rect}
  <circle cx="256" cy="256" r="{g['r']}" fill="none" stroke="{stroke}" stroke-width="{g['width']}"/>
  <path d="M{sx} {sy}A{g['arc']} {g['arc']} 0 0 0 {dx} {dy}" fill="none" stroke="{stroke}" stroke-width="{g['tail']}" stroke-linecap="round"/>
  <circle cx="{dx}" cy="{dy}" r="{g['rd']}" fill="{dot}"/>
</svg>
"""


def image(source, size):
    sized = source.replace("viewBox=", f'width="{size}" height="{size}" viewBox=', 1)
    return Rsvg.Handle.new_from_data(sized.encode()).get_pixbuf()


def enlarged(images, factor=8):
    """Legibility board: the images side by side, enlarged without smoothing."""
    size = images[0].get_width()
    board = GdkPixbuf.Pixbuf.new(GdkPixbuf.Colorspace.RGB, True, 8, size * factor * len(images), size * factor)
    board.fill(0xFFFFFFFF)
    for i, img in enumerate(images):
        img.scale(board, i * size * factor, 0, size * factor, size * factor,
                  i * size * factor, 0, factor, factor, GdkPixbuf.InterpType.NEAREST)
    return board


before, after = [], []
for name, colors in VARIANTS.items():
    # Without the square, the symbol fills its frame and reads at every size.
    source = svg(*colors, False, GEOMETRIES[""])
    with open(f"{name}-transparent.svg", "w") as f:
        f.write(source)
    for size in SIZES:
        image(source, size).savev(f"png/{name}-transparent-{size}.png", "png", [], [])
    # With the square, 16 and 32 px take their optical version.
    for suffix, g in GEOMETRIES.items():
        with open(f"{name}{suffix}.svg", "w") as f:
            f.write(svg(*colors, True, g))
    for size in SIZES:
        suffix = f"-{size}" if f"-{size}" in GEOMETRIES else ""
        source = svg(*colors, True, GEOMETRIES[suffix])
        image(source, size).savev(f"png/{name}-{size}.png", "png", [], [])
    before.append(image(svg(*colors, True, GEOMETRIES[""]), 16))
    after.append(image(svg(*colors, True, GEOMETRIES["-16"]), 16))
enlarged(before + after).savev("png/legibility-16.png", "png", [], [])
