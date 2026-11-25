# Thème clair

Ce dossier complète `docs/design/`. Dézippe-le à la racine de `docs/` : il ajoute `design/planches-clair/` et ce fichier, sans toucher aux planches sombres. Le dossier `images/` est le même que celui déjà présent.

## Ce qui change

L'OS a deux thèmes, sombre et clair. Ils ont la même structure, les mêmes tailles et les mêmes espacements. Seules les couleurs changent. Les planches claires sont les planches sombres avec les couleurs converties selon le tableau ci-dessous.

Conséquence pour le code : dans `desktop/sherlock-ui`, chaque couleur est un jeton nommé par son rôle (fond du bureau, texte principal, accent...), avec une valeur pour chaque thème. Aucun composant ne choisit une couleur selon le thème lui-même. Le thème se change dans les Réglages, et l'interface suit sans redémarrer.

## Planches

`planches-clair/` contient les seize écrans, avec le suffixe `Clair` : `MainClair.dc.html`, `DockClair.dc.html`, etc. Elles s'appellent par les mêmes adresses `/_blob/...` que les planches sombres pour les images de Sherlock. Le tableau de correspondance est dans `LISEZMOI.md`.

## Palette

### Fonds

| Rôle | Sombre | Clair |
| --- | --- | --- |
| Fond du bureau | `#0E1A2E` | `#E9EEF4` |
| Disques du fond d'écran | `#132A49`, `#16314F`, `#1B3F63` | `#DDE6F0`, `#D4E0EC`, `#C8D8E9` |
| Fond des écrans système | `#0A1424` | `#EEF2F6` |
| Disques des écrans système | `#0F2038`, `#11263E`, `#14304C` | `#E2E9F1`, `#D9E3EE`, `#D0DDEA` |
| Fond du mode jeu | `#080F1C` | `#E7EDF4` |
| Terminal, zones de code | `#0B1220` | `#F6F8FB` |
| Fenêtre | `#0F1828` | `#FFFFFF` |
| Barre de titre | `#131E32` | `#F3F5F9` |
| Panneau latéral | `#111B2E` | `#F6F8FB` |
| Carte | `#152238` | `#F3F6FA` |
| Bloc du terminal, carte de l'installeur | `#101B2E` | `#FFFFFF` |
| Verre des panneaux | `rgba(16,26,44,A)` | `rgba(255,255,255,A)`, même opacité |
| Verre des menus | `rgba(22,34,56,0.96)` | `rgba(255,255,255,0.97)` |
| Verre du dock | `rgba(244,235,227,0.17)` | `rgba(255,255,255,0.62)` |
| Teintes et bordures légères | `rgba(255,255,255,A)` | `rgba(20,35,59,A)`, opacité plafonnée à 0,16 |
| Ombres | `rgba(3,8,18,A)` | `rgba(20,35,59,A × 0,33)` |

### Texte et icônes

| Rôle | Sombre | Clair |
| --- | --- | --- |
| Texte principal | `#EEF2F7` | `#14233B` |
| Texte secondaire | `#C5CEDB` | `#46566F` |
| Texte tertiaire | `#A9B4C6` | `#5C6B82` |
| Numéros de ligne, texte fantôme | `#8492A8` | `#7A889D` |
| Texte du code | `#DCE4EE` | `#1E2B40` |
| Mots-clés du code, dossiers | `#8FC1E8` | `#1F64A0` |
| Types dans le code | `#E9DCC9` | `#7A4E12` |
| Vert, quand il colore du texte ou une icône | `#85C595` | `#2E7D4F` |
| Ambre, quand il colore du texte | `#F0C48A` | `#9A6412` |
| Saumon, quand il colore du texte | `#F6B5A6` | `#B23A2A` |

### Ce qui ne change pas

- Le vert `#85C595`, l'ambre `#F0C48A` et le saumon en **fond** : boutons principaux, barres de progression, pastilles.
- Le crème `#F4EBE3` de Sherlock et du système de confiance, avec son texte `#14233B`.
- Les fonds colorés des icônes du dock et leurs tracés.
- Les boutons bleu marine `#14233B` posés sur le crème.
- Le texte sombre `#0B1220` posé sur un fond vert.

## Règles propres au thème clair

- Une couleur vive ne sert jamais telle quelle pour du texte sur fond clair : on prend sa variante foncée du tableau. Le contraste minimum reste 4,5 pour 1.
- L'ombre portée sous Sherlock sur l'écran de connexion devient `rgba(20,35,59,0.22)`.
- Les reflets blancs en haut des icônes du dock restent blancs.

## Limites

Les planches claires ont été générées à partir des sombres. Quelques détails peuvent demander une retouche, en particulier les bulles crème de Sherlock, qui se détachent peu du fond clair. En cas de doute sur un écran, la palette de ce fichier fait foi.
