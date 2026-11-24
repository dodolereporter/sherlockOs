# Design de l'OS : planches et images

Ce dossier va dans `docs/design/` du dépôt. Il sert de référence visuelle aux agents. Ne le modifie pas.

## Planches

`planches/` contient les seize écrans du design, un fichier HTML par écran. Les styles sont écrits en ligne dans chaque balise : on y lit les couleurs, les tailles, les rayons et les espacements exacts.

Ces fichiers viennent d'un outil de design. Ils ne s'ouvrent pas tels quels dans un navigateur : la ligne `<script src="./support.js">` et les balises `<x-dc>`, `<dc-import>` et `<sc-if>` appartiennent à cet outil. Lis-les comme une spécification, pas comme du code à reprendre.

| Fichier | Écran |
| --- | --- |
| `Main.dc.html` | Bureau : barre du haut, éditeur, Conteneurs, panneau Sherlock, dock |
| `Dock.dc.html` | Dock, avec le calcul du grossissement dans le script en bas du fichier |
| `Sherlock.dc.html` | Sherlock, l'agent : demande, plan, fenêtre des accords, journal |
| `Labo.dc.html` | Labo : bacs à sable, outils, modèles de projet |
| `Fichiers.dc.html` | Fichiers : liste, aperçu, menu du clic droit et sous-menu |
| `Terminal.dc.html` | Terminal : invite à segments, blocs, aide de Sherlock, recettes |
| `Reglages.dc.html` | Réglages : page Jeu et carte graphique |
| `Menus.dc.html` | Menu Sherlock, menu Fenêtre, clic droit sur le bureau et dans le dock |
| `Panneaux.dc.html` | Lanceur, centre de notifications, réglages rapides |
| `Connexion.dc.html` | Écran de connexion |
| `Jeu.dc.html` | Mode jeu |
| `MenuBoot.dc.html` | Menu de démarrage GRUB |
| `Demarrage.dc.html` | Écran de démarrage |
| `InstalleurDisque.dc.html` | Installeur, étape 2 : disque |
| `InstalleurProfil.dc.html` | Installeur, étape 4 : profil |
| `InstalleurInstallation.dc.html` | Installeur, étape 6 : installation |

## Images de Sherlock

Dans les planches, les images sont appelées par une adresse `/_blob/<identifiant>`. Voici la correspondance avec les fichiers de `images/`.

| Adresse dans les planches | Fichier | Pose |
| --- | --- | --- |
| `/_blob/16a817327778fb99b1a30342b0a01682` | `images/sherlock-neutre.png` | Debout, écharpe verte |
| `/_blob/c2a7ff899d17e97f55b53460c95a8516` | `images/sherlock-loupe.png` | Avec sa loupe |
| `/_blob/f1ffff9bc58dac215d9dfcd8ae6adf75` | `images/sherlock-content.png` | Yeux fermés, content |
| `/_blob/5a8a7b7276da27a1a4cc419bd21d291a` | `images/sherlock-tete.png` | Tête seule, pour les icônes |

Ces images sont des découpes d'une planche du dépôt `dodolereporter/sherlock`. Elles servent aux maquettes. Dans l'OS, Sherlock est rendu en 3D à partir de `sherlock.glb`.

## Données d'exemple

Les noms de conteneurs, de fichiers, les heures et les textes des planches sont des exemples. Les mentions entre crochets, comme `[NOM DE L'OS]`, marquent une information qui n'existe pas encore.
