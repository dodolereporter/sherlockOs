# ADR 0007 : Mode jeu en plein écran géré par notre compositeur

- Statut : accepté
- Date : 2026-11-24
- Lot : plan, tableau « Décisions figées »

## Contexte

Le mode jeu doit afficher un jeu en plein écran avec le moins de latence possible. gamescope fait ce travail ailleurs, mais un ticket chez Valve (n° 1662) décrit un plantage sur NVIDIA en session Wayland imbriquée.

## Décision

En 0.1, le plein écran du mode jeu est géré par notre compositeur, avec affichage direct sans composition. gamescope reste une option pour plus tard.

## Conséquences

- Le compositeur doit gérer la synchronisation explicite et l'affichage direct (lot D-15).
- La session mode jeu (lot J-16) s'appuie sur notre compositeur.

## Ce qui ferait changer d'avis

- gamescope devient stable sur NVIDIA sous Wayland.
- Notre affichage direct ne tient pas les mesures du lot J-17.
