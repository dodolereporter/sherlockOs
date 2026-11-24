# ADR 0010 : Bacs à sable construits sur Docker

- Statut : accepté
- Date : 2026-11-24
- Lot : plan, tableau « Décisions figées »

## Contexte

Les bacs à sable sont des espaces jetables où l'on essaie du code ou une commande, avec le réseau coupé et le dossier personnel caché. Ils servent aussi aux essais à blanc de Sherlock.

## Décision

En 0.1, les bacs à sable sont construits sur Docker.

## Conséquences

- Un seul moteur à maintenir pour les conteneurs et les bacs à sable.
- L'isolation dépend de Docker : le lot L-12 doit prouver qu'un script ne sort ni sur le réseau ni vers le dossier personnel.

## Ce qui ferait changer d'avis

- Les tests de L-12 ou la revue de sécurité A-20 montrent une isolation insuffisante.
