# ADR 0008 : Steam tourne en silence derrière notre app Jeux

- Statut : accepté
- Date : 2026-11-24
- Lot : plan, tableau « Décisions figées »

## Contexte

Le mode jeu montre la bibliothèque et le catalogue Steam dans notre interface. Réécrire les téléchargements, Proton ou la gestion des manettes serait hors de portée.

## Décision

Le client Steam tourne en arrière-plan, sans fenêtre. Notre app lit la bibliothèque dans les fichiers de Steam (`libraryfolders.vdf`, puis un `appmanifest` par jeu) et lance un jeu par l'adresse `steam://rungameid/`. Le catalogue est en lecture seule et l'achat ouvre la page de Steam.

## Conséquences

- Téléchargements, Proton et manettes restent gérés par Steam.
- La lecture des fichiers de Steam vit dans un module isolé, avec un repli sur l'ouverture du client Steam.
- Le choix de reprendre le projet reSteam pour l'app Jeux reste une question ouverte.

## Ce qui ferait changer d'avis

- Steam change le format de ses fichiers et la bibliothèque devient illisible.
- Steam empêche de faire tourner son client sans fenêtre.
