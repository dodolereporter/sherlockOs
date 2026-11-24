# ADR 0005 : Shell en programmes séparés

- Statut : accepté
- Date : 2026-11-24
- Lot : plan, tableau « Décisions figées »

## Contexte

La barre, le dock, le lanceur et les notifications pourraient vivre dans le compositeur. Un plantage de l'un d'eux emporterait alors toute la session.

## Décision

Chaque élément du shell est un programme séparé, ancré à l'écran par le protocole layer-shell. S'il plante, la session continue et il redémarre.

## Conséquences

- Le compositeur doit prendre en charge layer-shell (lot D-12).
- Chaque programme a son crate dans `desktop/` et se teste seul.
- Les échanges entre le shell et le compositeur passent par des protocoles Wayland ou une API explicite.
- Exception : la fenêtre des accords de Sherlock est dessinée par le compositeur lui-même (ADR 0011).

## Ce qui ferait changer d'avis

- Le coût des échanges entre processus empêche de tenir 60 images par seconde, par exemple sur le grossissement du dock.
