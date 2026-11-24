# ADR 0002 : Base glibc, systemd et pacman

- Statut : accepté
- Date : 2026-11-24
- Lot : plan, tableau « Décisions figées »

## Contexte

Le système est construit depuis les sources, sans dériver d'une distribution existante. Il faut choisir la bibliothèque C, le système d'init et le gestionnaire de paquets. Steam exige glibc. Docker, les sessions utilisateur et les journaux reposent couramment sur systemd.

## Décision

La base utilise glibc, systemd, pacman et makepkg. Les recettes de paquets s'inspirent de celles d'Arch Linux, sans les copier aveuglément.

## Conséquences

- Steam, Proton et les bibliothèques 32 bits tournent sans couche de compatibilité.
- systemd fournit les services, les sessions utilisateur et le journal que lisent Sherlock et les Réglages.
- Le dépôt de paquets doit être signé et maintenu (lots S-12 et Q-06).

## Ce qui ferait changer d'avis

- Une dépendance de Steam ou de Docker devient incompatible avec cette base.
- La maintenance des paquets dépasse ce qu'un auteur seul peut suivre, selon le rapport de veille Q-06.
