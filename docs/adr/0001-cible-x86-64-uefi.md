# ADR 0001 : Cible x86_64 et UEFI

- Statut : accepté
- Date : 2026-11-24
- Lot : plan, tableau « Décisions figées »

## Contexte

Le projet vise une 0.1 utilisée tous les jours sur une seule machine : le portable de l'auteur, Ryzen 7 avec sa puce AMD intégrée et une RTX 3060, démarré en UEFI. Viser plusieurs architectures dès le départ multiplierait les builds et les tests sans servir cet objectif.

## Décision

La seule cible est x86_64 en UEFI. Le portable de l'auteur sert de machine de référence. Le système reste conçu de façon générique : tout passe par la détection du matériel, rien n'est écrit pour cette machine en particulier.

## Conséquences

- Une seule toolchain et une seule image à construire et à tester.
- Le démarrage BIOS, ARM et Secure Boot sont hors périmètre de la 0.1.
- Les niveaux de compatibilité du plan s'appliquent : référence, générique, au mieux. Le mot « compatible » ne s'écrit que pour ce qui a été testé.

## Ce qui ferait changer d'avis

- L'auteur change de machine principale.
- Une seconde machine de test devient disponible et demande une autre architecture.
