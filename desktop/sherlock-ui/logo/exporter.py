#!/usr/bin/env python3
"""Écrit les six SVG du logo et leurs PNG. Lancer depuis ce dossier : python3 exporter.py

Le symbole est dessiné sur une grille de 512 : anneau de rayon 140 et de 36 d'épaisseur centré
en (256, 256), queue en arc de rayon 80 et de 28 d'épaisseur qui part de l'anneau en (340, 368)
et finit au point de rayon 22 centré en (308, 256). Carré de fond de 512, rayon 96.
Les PNG passent par librsvg (paquets Debian gir1.2-rsvg-2.0 et python3-gi).
"""
import gi

gi.require_version("Rsvg", "2.0")
from gi.repository import Rsvg  # noqa: E402

NUIT, VERT, CREME = "#0E1A2E", "#85C595", "#F4EBE3"
# Nom : (fond du carré, anneau et queue, point)
VARIANTES = {
    "logo": (NUIT, VERT, CREME),
    "logo-creme": (NUIT, CREME, CREME),
    "logo-nuit": (CREME, NUIT, NUIT),
}
TAILLES = (16, 32, 64, 256, 512)


def svg(fond, trait, point, carre):
    boite = "0 0 512 512" if carre else "96 96 320 320"
    rect = f'\n  <rect width="512" height="512" rx="96" fill="{fond}"/>' if carre else ""
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="{boite}">{rect}
  <circle cx="256" cy="256" r="140" fill="none" stroke="{trait}" stroke-width="36"/>
  <path d="M340 368A80 80 0 0 0 308 256" fill="none" stroke="{trait}" stroke-width="28" stroke-linecap="round"/>
  <circle cx="308" cy="256" r="22" fill="{point}"/>
</svg>
"""


for nom, couleurs in VARIANTES.items():
    for carre, suffixe in ((True, ""), (False, "-sans-fond")):
        source = svg(*couleurs, carre)
        with open(f"{nom}{suffixe}.svg", "w") as f:
            f.write(source)
        for taille in TAILLES:
            dimensionne = source.replace("viewBox=", f'width="{taille}" height="{taille}" viewBox=', 1)
            image = Rsvg.Handle.new_from_data(dimensionne.encode()).get_pixbuf()
            image.savev(f"png/{nom}{suffixe}-{taille}.png", "png", [], [])
