#!/usr/bin/env python3
"""Écrit les SVG du logo et leurs PNG. Lancer depuis ce dossier : python3 exporter.py

Le symbole est dessiné sur une grille de 512 : anneau centré en (256, 256), queue en arc qui
part de l'anneau et finit au point, carré de fond de 512 au rayon 96. Les versions optiques
pour 32 et 16 px épaississent l'anneau, agrandissent le symbole dans le carré et raccourcissent
la queue, pour que le dessin tienne sur si peu de pixels. Les PNG passent par librsvg (paquets
Debian gir1.2-rsvg-2.0 et python3-gi).
"""
import gi

gi.require_version("Rsvg", "2.0")
gi.require_version("GdkPixbuf", "2.0")
from gi.repository import GdkPixbuf, Rsvg  # noqa: E402

NUIT, VERT, CREME = "#0E1A2E", "#85C595", "#F4EBE3"
# Nom : (fond du carré, anneau et queue, point)
VARIANTES = {
    "logo": (NUIT, VERT, CREME),
    "logo-creme": (NUIT, CREME, CREME),
    "logo-nuit": (CREME, NUIT, NUIT),
}
# Géométries, en unités de la grille de 512 : rayon et épaisseur de l'anneau, départ de la
# queue sur l'anneau (triangle 3-4-5 : à 53° sous l'horizontale), point d'arrivée, rayon et
# épaisseur de la queue, rayon du point.
GEOMETRIES = {
    "": dict(r=140, ep=36, depart=(340, 368), point=(308, 256), arc=80, queue=28, rp=22),
    # 32 px : 1 px vaut 16 unités. Anneau de 3 px, symbole agrandi.
    "-32": dict(r=175, ep=50, depart=(361, 396), point=(300, 248), arc=100, queue=36, rp=36),
    # 16 px : 1 px vaut 32 unités. Anneau de 1,75 px, marge de 1,5 px, queue courte et épaisse.
    "-16": dict(r=180, ep=56, depart=(364, 400), point=(300, 252), arc=110, queue=40, rp=38),
}
TAILLES = (16, 32, 64, 256, 512)


def svg(fond, trait, point, carre, g):
    boite = "0 0 512 512" if carre else "96 96 320 320"
    rect = f'\n  <rect width="512" height="512" rx="96" fill="{fond}"/>' if carre else ""
    (dx, dy), (px, py) = g["depart"], g["point"]
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="{boite}">{rect}
  <circle cx="256" cy="256" r="{g['r']}" fill="none" stroke="{trait}" stroke-width="{g['ep']}"/>
  <path d="M{dx} {dy}A{g['arc']} {g['arc']} 0 0 0 {px} {py}" fill="none" stroke="{trait}" stroke-width="{g['queue']}" stroke-linecap="round"/>
  <circle cx="{px}" cy="{py}" r="{g['rp']}" fill="{point}"/>
</svg>
"""


def image(source, taille):
    dimensionne = source.replace("viewBox=", f'width="{taille}" height="{taille}" viewBox=', 1)
    return Rsvg.Handle.new_from_data(dimensionne.encode()).get_pixbuf()


def agrandi(images, facteur=8):
    """Planche de lisibilité : les images côte à côte, agrandies sans lissage."""
    taille = images[0].get_width()
    planche = GdkPixbuf.Pixbuf.new(GdkPixbuf.Colorspace.RGB, True, 8, taille * facteur * len(images), taille * facteur)
    planche.fill(0xFFFFFFFF)
    for i, img in enumerate(images):
        img.scale(planche, i * taille * facteur, 0, taille * facteur, taille * facteur,
                  i * taille * facteur, 0, facteur, facteur, GdkPixbuf.InterpType.NEAREST)
    return planche


avant, apres = [], []
for nom, couleurs in VARIANTES.items():
    # Sans carré, le symbole remplit son cadre et se lit à toutes les tailles.
    source = svg(*couleurs, False, GEOMETRIES[""])
    with open(f"{nom}-sans-fond.svg", "w") as f:
        f.write(source)
    for taille in TAILLES:
        image(source, taille).savev(f"png/{nom}-sans-fond-{taille}.png", "png", [], [])
    # Avec carré, 16 et 32 px prennent leur version optique.
    for suffixe, g in GEOMETRIES.items():
        with open(f"{nom}{suffixe}.svg", "w") as f:
            f.write(svg(*couleurs, True, g))
    for taille in TAILLES:
        suffixe = f"-{taille}" if f"-{taille}" in GEOMETRIES else ""
        source = svg(*couleurs, True, GEOMETRIES[suffixe])
        image(source, taille).savev(f"png/{nom}-{taille}.png", "png", [], [])
    avant.append(image(svg(*couleurs, True, GEOMETRIES[""]), 16))
    apres.append(image(svg(*couleurs, True, GEOMETRIES["-16"]), 16))
agrandi(avant + apres).savev("png/lisibilite-16.png", "png", [], [])
