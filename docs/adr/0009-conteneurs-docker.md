# ADR 0009 : Docker Engine et Compose d'origine

- Statut : accepté
- Date : 2026-11-24
- Lot : plan, tableau « Décisions figées »

## Contexte

Le projet veut un OS fait pour développer, avec des conteneurs disponibles dès l'installation. C'est une demande explicite de l'auteur.

## Décision

Docker Engine et Docker Compose sont livrés en paquets et actifs au démarrage.

## Conséquences

- La base doit fournir les cgroups v2 et les prérequis du noyau (lot S-18).
- Le Labo, les bacs à sable et le serveur MCP conteneurs s'appuient sur l'API Docker.
- Les conteneurs sont mis en pause pendant une session de jeu.

## Ce qui ferait changer d'avis

- L'auteur change d'outil de conteneurs pour son travail quotidien.
