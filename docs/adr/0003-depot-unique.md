# ADR 0003 : Un seul dépôt

- Statut : accepté
- Date : 2026-11-24
- Lot : plan, tableau « Décisions figées »

## Contexte

Le projet couvre la distribution, le desktop et l'agent. Ils évoluent ensemble et sont codés par des agents qui doivent voir l'ensemble pour ne pas casser une brique voisine.

## Décision

Un seul dépôt, découpé en `distro/`, `desktop/`, `sherlock/` et `docs/`. Les crates Rust forment un seul workspace Cargo. Les prototypes jetables vivent dans `essais/`, hors du workspace.

## Conséquences

- Une seule CI vérifie tout le projet à chaque demande de fusion.
- Un agent lit le contexte, le plan et les ADR au même endroit.
- La CI grossit avec le projet : les jobs lourds (image, QEMU) devront ne se lancer que sur les dossiers concernés.

## Ce qui ferait changer d'avis

- La durée de la CI rend les demandes de fusion pénibles malgré le filtrage par dossier.
- Une brique doit être publiée seule, avec une licence ou une visibilité différente.
