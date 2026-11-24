# ADR 0012 : Modèle IA par API cloud, derrière une interface

- Statut : accepté
- Date : 2026-11-24
- Lot : plan, tableau « Décisions figées »

## Contexte

Sherlock a besoin d'un modèle de langage. Un modèle local demande du matériel et du réglage qui retarderaient la 0.1. Le fournisseur n'est pas encore choisi.

## Décision

Sherlock appelle d'abord une API cloud, derrière une interface Rust de fournisseur de modèle. Un modèle local viendra plus tard, derrière la même interface. La clé vit dans le trousseau de l'utilisateur, jamais dans le dépôt.

## Conséquences

- On change de fournisseur sans toucher à l'agent.
- Les secrets (`.env`, clés, jetons) sont masqués avant tout envoi au modèle.
- Sherlock a besoin du réseau tant qu'aucun modèle local n'existe.

## Ce qui ferait changer d'avis

- Un modèle local tient la qualité requise sur la machine de référence.
- Le fournisseur choisi impose des conditions incompatibles avec le projet.
