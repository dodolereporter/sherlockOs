# ADR 0013 : L'éditeur, le navigateur et le moteur de terminal sont empaquetés

- Statut : accepté
- Date : 2026-11-24
- Lot : plan, tableau « Décisions figées »

## Contexte

Un éditeur de code, un navigateur et un émulateur de terminal complets représentent chacun des années de travail. Ils ne servent pas l'objectif de la 0.1.

## Décision

On empaquette un éditeur et un navigateur existants. Le terminal a une interface maison, mais son moteur d'émulation est repris d'un projet existant.

## Conséquences

- La fenêtre Éditeur du design est une illustration, pas un lot.
- Le lot L-15 choisit le moteur d'émulation et le justifie.

## Ce qui ferait changer d'avis

- Aucun moteur d'émulation existant ne permet les blocs de commandes et l'invite à segments de la planche Terminal.
