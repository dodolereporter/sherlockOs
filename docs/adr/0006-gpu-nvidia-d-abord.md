# ADR 0006 : NVIDIA d'abord, modules ouverts, pilote épinglé

- Statut : accepté
- Date : 2026-11-24
- Lot : plan, tableau « Décisions figées »

## Contexte

La machine de référence est un portable hybride. L'écran interne est câblé sur la puce AMD, la RTX 3060 sert au rendu des jeux. Le pilote NVIDIA sous Wayland reste la principale source d'instabilité identifiée par le plan.

## Décision

Le réglage pour le jeu vise d'abord NVIDIA, avec les modules noyau ouverts et une version du pilote épinglée. Mesa est requis pour la puce AMD qui pilote l'écran interne, et pour la CI dans QEMU.

## Conséquences

- Une montée de version du pilote passe d'abord le test d'une heure du lot J-11.
- Les cartes NVIDIA plus anciennes que les RTX 20 reçoivent le pilote libre, sans optimisation pour le jeu.
- Le réglage fin sur AMD et Intel est repoussé après la 0.1. L'OS doit tout de même y démarrer.

## Ce qui ferait changer d'avis

- Les modules ouverts ne gèrent pas une fonction nécessaire sur la version retenue.
- Une version épinglée devient incompatible avec le noyau de la base.
