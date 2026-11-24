# Plan détaillé

La version 0.1 vise ton desktop en Rust, avec Sherlock, Docker natif et un mode jeu réglé pour NVIDIA, sur ton propre système et sur ta machine. La fusion est visée vers 12 mois, en douze jalons.

## Périmètre de la 0.1

La 0.1 est faite pour une seule machine, la tienne. Tout ce qui n'aide pas à y arriver est repoussé.

**Dedans**

- Une machine de référence : ton portable, Ryzen 7 et RTX 3060, en x86\_64 et UEFI.
- Le système construit depuis les sources : noyau, glibc, systemd, pacman, pile graphique, bibliothèques 32 bits, menu GRUB, écran de démarrage et installeur à notre design.
- Le desktop en Rust : compositeur, barre, dock, lanceur, notifications, écran de connexion, verrouillage, réglages, fichiers, terminal.
- Sherlock : l'agent, ses serveurs MCP, les accords, le journal et la mascotte 3D animée.
- Le Labo : Docker natif, bacs à sable, outils de dev préinstallés.
- Le mode jeu : bibliothèque et catalogue Steam dans notre interface, Proton, profil NVIDIA.

**Repoussé**

- Le réglage fin des jeux sur les cartes AMD et Intel. L'OS démarre et fonctionne dessus, mais seul NVIDIA est optimisé en 0.1.
- Un éditeur de code et un navigateur maison. On empaquette l'existant. Le terminal est à nous, seul son moteur d'émulation est repris. La fenêtre Éditeur du design est une illustration.
- Un toolkit d'interface maison.
- Le HDR, Secure Boot, ARM.
- L'achat de jeux dans notre interface (il passe par Steam) et les jeux à anti-triche noyau.

## Architecture

&#91;embedded content: architecture · 5 couches, Sherlock sur le côté\]

Chaque couche expose ses fonctions par un serveur MCP. L'interface et Sherlock appellent la même API, et toute action de Sherlock passe par le courtier d'accords. Le dépôt suit ce découpage : `distro/` pour la base et les services, `desktop/` pour le compositeur, le shell et les applications, `sherlock/` pour l'agent et ses serveurs.

## Décisions figées

Ces choix ne se rediscutent pas en cours de lot. Pour en changer un, on écrit un ADR (une fiche de décision) et on ajuste le plan.

| Sujet | Décision | Raison |
| --- | --- | --- |
| Cible | x86\_64, UEFI. Ton portable (Ryzen 7, RTX 3060) sert de référence | Conçu générique, validé d'abord sur une seule machine |
| Base | glibc, systemd, pacman | Steam exige glibc. systemd simplifie Docker, les sessions et les journaux |
| Dépôt | Un seul dépôt : `distro/`, `desktop/`, `sherlock/`, `docs/` | Une seule CI, des agents qui voient l'ensemble |
| Desktop | Rust, Smithay, iced | Voie déjà suivie par COSMIC. iced reste à confirmer par le lot D-00 |
| Shell | Barre, dock, lanceur et notifications en programmes séparés | Un plantage n'emporte pas la session |
| GPU | NVIDIA d'abord, modules noyau ouverts, version du pilote épinglée | L'écran interne de ton portable passe par la puce AMD, donc Mesa est requis |
| Mode jeu | Plein écran géré par notre compositeur. gamescope en option, plus tard | gamescope reste capricieux sur NVIDIA |
| Steam | Le client Steam tourne en silence. Notre app lit la bibliothèque et lance les jeux | Téléchargements, Proton et manettes restent gérés par Steam |
| Conteneurs | Docker Engine et Compose en paquets, actifs au démarrage | Demande du projet |
| Bacs à sable | Construits sur Docker pour la 0.1 | Un seul moteur à maintenir |
| Sherlock | Un démon, un serveur MCP par domaine, un courtier d'accords dans le compositeur | L'agent ne peut pas s'accorder lui-même une action |
| Modèle IA | API cloud derrière une interface, modèle local plus tard | Changer de fournisseur sans toucher à l'agent |
| Existant empaqueté | Éditeur, navigateur, moteur de terminal | Hors périmètre de la 0.1 |

## Jalons

Un jalon est atteint quand son critère de sortie est vérifié, pas quand ses lots sont codés. Les mois sont des estimations en soirées et week-ends. J1 à J4 se font sur ta Debian, J5 avance en parallèle.

| Jalon | Contenu | Critère de sortie | Mois visé |
| --- | --- | --- | --- |
| J0 Fondations | Dépôt, CI, conteneur de build, banc de test QEMU, choix du toolkit | La CI est verte et une image minimale boote dans QEMU sans intervention | 0,5 |
| J1 Compositeur imbriqué | Fenêtres, déplacement, coins, ombres, flou, XWayland | Un navigateur et un terminal tournent dans le compositeur, en fenêtre sur Debian | 2 |
| J2 Shell | Barre, dock, lanceur, notifications, bibliothèque de composants | Le bureau du design est reproduit, dock animé compris | 4 |
| J3 Session réelle | DRM, connexion, verrouillage, portails, réglages, fichiers | Tu travailles une semaine dessus sans revenir à Plasma | 6 |
| J4 Sherlock v1 | Démon, accords, journal, cinq serveurs MCP, mascotte animée | Le scénario « relance front-next » marche de bout en bout, accord compris | 7 |
| J5 Base bootable | Toolchain, noyau, systemd, pacman, fabrique d'images | L'image se reconstruit de zéro en CI et accepte une mise à jour pacman | 5 |
| J6 Desktop sur notre base | Mesa, Wayland, PipeWire, réseau, notre session par défaut | Le desktop démarre sur notre base dans QEMU, puis sur ta machine | 8 |
| J7 NVIDIA, Steam, Proton | Pilote, 32 bits, Steam, manettes | Trois de tes jeux Proton se lancent sur la carte NVIDIA | 10 |
| J8 Mode jeu | App Jeux, session plein écran, profil NVIDIA, pause des conteneurs | Du démarrage au jeu à la manette, sans toucher au clavier | 11 |
| J9 Labo | Docker natif, bacs à sable, outils, modèles de projet | Un bac à sable se crée, se fige et se jette. Un projet Spring et Postgres démarre en un clic | 11 |
| J10 Fusion 0.1 | Installation en double boot, mises à jour, tests matériels | Deux semaines d'usage quotidien sans retour à Debian | 12 |

Un douzième jalon suit la fusion. **J11, démarrage et installeur**, vers le mois 13 : menu GRUB et écran de démarrage à notre design, installeur graphique (lots S-19 à S-21). Il est atteint quand une installation complète à côté de Debian réussit sur ta machine sans ligne de commande.

Après J11 vient la finition, que j'estime à 6 à 12 mois de plus pour le niveau visé.

## Lots de travail

Un lot tient dans une demande de fusion et se vérifie par une commande ou un test. La colonne « Accepté quand » est le critère que l'agent doit prouver. Les lots sans dépendance commune peuvent tourner en parallèle.

### Piste Q : qualité et fondations

| Lot | Contenu | Dépend de | Accepté quand |
| --- | --- | --- | --- |
| Q-01 | Squelette du dépôt, CLAUDE.md, modèle d'ADR, version de Rust épinglée | Aucun | `cargo build --workspace` passe sur un poste vierge |
| Q-02 | CI Rust : format, clippy sans avertissement, tests | Q-01 | Une demande de fusion avec un avertissement est refusée |
| Q-03 | Conteneur de build de la distribution | Q-01 | Le build ne dépend de rien d'autre que du conteneur |
| Q-04 | Banc de test QEMU : boot sans écran, marqueur sur le port série, capture d'écran | Q-03 | La CI échoue si l'image ne boote pas en 90 secondes |
| Q-05 | Tests visuels du desktop : captures comparées à une référence | D-10 | Une régression de rendu fait échouer la CI |
| Q-06 | Veille hebdomadaire des versions et des failles des paquets | S-12 | Un rapport liste les paquets en retard |

### Piste D : desktop

Tout se développe et se teste sur Debian, dans une fenêtre puis en session réelle.

| Lot | Contenu | Dépend de | Accepté quand |
| --- | --- | --- | --- |
| D-00 | Essai iced contre Slint : un panneau ancré à l'écran, avec flou | Q-01 | L'ADR est écrit et le toolkit choisi |
| D-01 | `sherlock-ui` : couleurs, rayons, typographie et composants de base tirés du design | D-00 | La galerie de composants correspond au canvas |
| D-10 | Compositeur minimal : fenêtres, clavier, souris, déplacer, redimensionner | Q-01 | Un terminal tourne dedans, en fenêtre sur Debian |
| D-11 | Rendu : coins arrondis, ombres, flou, animations à ressorts | D-10 | 60 images par seconde tenues avec dix fenêtres |
| D-12 | Surfaces ancrées (layer-shell), multi-écran, mise à l'échelle fractionnaire | D-10 | Une barre de test reste en place sur deux écrans |
| D-13 | XWayland | D-10 | Une application X11 s'affiche et échange le presse-papiers |
| D-14 | Session réelle : DRM, libseat, changement de console | D-11 | Le compositeur démarre depuis une console sur ta machine |
| D-15 | Synchronisation explicite et affichage direct en plein écran | D-14 | Le protocole est annoncé et un client plein écran évite la composition |
| D-20 | Barre du haut | D-01, D-12 | Conforme au canvas, horloge et bureaux actifs |
| D-21 | Dock avec grossissement | D-01, D-12 | Conforme au canvas, fluide à 60 images par seconde |
| D-22 | Lanceur et recherche | D-01 | Une application se lance au clavier en moins de trois touches |
| D-23 | Notifications | D-01 | Une notification standard s'affiche et se ferme |
| D-24 | Écran de connexion sur greetd | D-01, D-14 | Ouverture de session, choix Bureau ou Mode jeu |
| D-25 | Verrouillage de session | D-14 | Aucune fenêtre n'est visible ni cliquable une fois verrouillé |
| D-26 | Réglages : écrans, son, réseau, apparence | D-01 | Chaque réglage survit à un redémarrage |
| D-27 | Fichiers | D-01 | Copier, déplacer, renommer, corbeille, aperçu |
| D-28 | Portails : sélecteur de fichiers, capture, partage d'écran | D-14 | Un navigateur partage l'écran en visio |
| D-29 | Presse-papiers, glisser-déposer, saisie des accents | D-13 | Les échanges marchent entre applications Wayland et X11 |
| D-30 | Mascotte 3D : rendu de `sherlock.glb` et machine à états des animations | D-01 | Chaque état de l'agent joue le bon clip, avec fondu |

### Piste S : système

Cette piste tourne en tâche de fond dès J0. Chaque lot se vérifie dans QEMU.

| Lot | Contenu | Dépend de | Accepté quand |
| --- | --- | --- | --- |
| S-10 | Toolchain et système de base, entièrement scriptés | Q-03 | Un système minimal se reconstruit de zéro dans le conteneur |
| S-11 | Noyau, initramfs, GRUB | S-10, Q-04 | Une invite de connexion apparaît dans QEMU |
| S-12 | pacman et makepkg, base refaite en paquets, dépôt signé | S-11 | `pacman -Syu` met l'image à jour |
| S-13 | systemd, réseau, utilisateurs, français et clavier AZERTY | S-12 | Le réseau monte seul et la session s'ouvre en français |
| S-14 | Pile graphique : Mesa, Wayland, libinput, polices, PipeWire | S-12 | Un compositeur de référence s'affiche dans QEMU, avec le son |
| S-15 | Fabrique d'images : disque et clé d'installation | S-12 | Une commande produit une image qui boote |
| S-16 | Multilib : toolchain et bibliothèques 32 bits | S-14 | Un programme OpenGL 32 bits tourne |
| S-17 | Notre desktop en paquets, session par défaut | S-14, jalon J3 | Le jalon J6 est atteint |
| S-18 | Docker Engine, Compose, cgroups v2 | S-13 | `docker run hello-world` marche au premier démarrage |

Trois lots ajoutés pour le démarrage et l'installation, à partir des planches du canvas :

| Lot | Contenu | Dépend de | Accepté quand |
| --- | --- | --- | --- |
| S-19 | Thème GRUB : fond, entrées, compte à rebours, entrées mode jeu, console de secours et noyau précédent | S-11 | Le menu correspond à la planche dans QEMU et chaque entrée démarre |
| S-20 | Écran de démarrage animé avec Sherlock | S-14 | Aucun texte du noyau n'apparaît entre GRUB et l'écran de connexion, Échap affiche le détail |
| S-21 | Installeur graphique en six étapes : langue, disque, compte, profil, résumé, installation | S-15, D-01 | Une installation à côté d'un Debian existant réussit dans QEMU et les deux systèmes démarrent |

### Piste J : jeu et NVIDIA

J-14 et J-15 ne dépendent pas du système. Ils peuvent commencer sur Debian dès le jalon J2.

| Lot | Contenu | Dépend de | Accepté quand |
| --- | --- | --- | --- |
| J-10 | Paquets du pilote NVIDIA : modules ouverts pour notre noyau, bibliothèques 64 et 32 bits | S-16 | `nvidia-smi` répond sur ta machine |
| J-11 | Compositeur sur NVIDIA : synchronisation explicite, fréquence variable, bascule AMD et NVIDIA | D-15, J-10 | Une heure de test sans scintillement ni fuite mémoire |
| J-12 | Steam : paquet, règles des manettes, espaces de noms utilisateur | S-16 | Le client démarre et télécharge un jeu |
| J-13 | Proton : matrice de test sur tes jeux, cache de shaders | J-11, J-12 | Trois jeux validés, résultats notés dans le dépôt |
| J-14 | App Jeux, bibliothèque : lecture des fichiers Steam, jaquettes, lancement, navigation à la manette | D-01 | Tes jeux installés s'affichent et se lancent |
| J-15 | App Jeux, catalogue : boutique Steam en lecture, achat renvoyé vers Steam | J-14 | Une recherche renvoie une fiche de jeu avec son prix |
| J-16 | Session mode jeu : plein écran, notifications coupées, conteneurs en pause, limiteur d'images | J-11, J-14, S-18 | Le jalon J8 est atteint |
| J-17 | Profil NVIDIA : réglages du pilote, énergie, options Proton | J-13 | Des mesures avant et après sur trois jeux sont publiées |

### Piste A : Sherlock

L'agent se développe sur Debian. Le courtier d'accords passe avant tout serveur qui agit.

| Lot | Contenu | Dépend de | Accepté quand |
| --- | --- | --- | --- |
| A-10 | `sherlockd` : boucle d'agent, client MCP, interface de fournisseur de modèle | Q-01 | L'agent répond avec un serveur MCP de test |
| A-11 | Courtier d'accords : trois niveaux, fenêtre dessinée par le compositeur, journal | D-10, A-10 | Un test prouve qu'aucune action ne s'exécute sans accord |
| A-12 | Serveur MCP fenêtres | D-10 | Lister, déplacer et ranger les fenêtres |
| A-13 | Serveur MCP fichiers, limité aux dossiers autorisés | A-11 | Un accès hors dossier autorisé est refusé et journalisé |
| A-14 | Serveur MCP conteneurs, sur l'API Docker | A-11 | Le scénario « relance front-next » passe |
| A-15 | Serveur MCP paquets : lire, planifier, appliquer | A-11, S-12 | L'installation de Steam passe par un accord |
| A-16 | Serveur MCP services et journaux système | A-11 | Sherlock explique un service en échec à partir de ses logs |
| A-17 | Serveur MCP réglages, réseau, son | D-26 | Chaque réglage de l'interface est aussi pilotable par l'agent |
| A-18 | Serveur MCP jeux | J-14 | Sherlock lance un jeu et lit ses logs Proton |
| A-19 | Interface : panneau « Demande à Sherlock », plan, accords | D-22, A-11 | Conforme à la planche Sherlock du canvas |
| A-20 | Revue de sécurité : consignes cachées dans des fichiers ou des pages, essais à blanc en bac à sable | A-13 à A-16, L-12 | Aucune consigne cachée ne déclenche d'action dans la suite de tests |

### Piste L : Labo

L-10 à L-12 se développent sur Debian, avec le Docker déjà installé.

| Lot | Contenu | Dépend de | Accepté quand |
| --- | --- | --- | --- |
| L-10 | App Labo : coquille et navigation | D-01 | Conforme à la planche Labo du canvas |
| L-11 | Conteneurs : liste, démarrer, arrêter, logs | L-10 | Les conteneurs de ta machine s'affichent et se pilotent |
| L-12 | Bacs à sable : créer, figer, jeter, réseau coupé | L-11 | Un script lancé dedans ne peut ni sortir sur le réseau ni lire ton dossier personnel |
| L-13 | Outils : Rust, Java, Node et Python en paquets, version par projet | S-12 | Deux projets utilisent deux versions de Java sans conflit |
| L-14 | Modèles de projet | L-12 | Le modèle Spring et Postgres démarre en un clic |
| L-15 | Terminal maison : invite à segments (dossier, git, langage, conteneurs, durée), blocs de commandes, recettes du projet, aide de Sherlock sur les erreurs. Moteur d'émulation existant | D-01 | Conforme à la planche Terminal du canvas, et une suite de tests d'affichage standard passe |

### Lots ajoutés : portable et compatibilité

| Lot | Contenu | Dépend de | Accepté quand |
| --- | --- | --- | --- |
| D-16 | Multi-GPU : compositeur sur la puce intégrée, jeux rendus par la carte dédiée, écrans externes câblés sur la carte dédiée | D-14 | Sur ton portable, l'écran interne et un écran externe marchent ensemble |
| D-31 | Pavé tactile : gestes à plusieurs doigts, défilement naturel, frappe involontaire ignorée | D-10 | On change de bureau à trois doigts |
| D-32 | Batterie dans la barre et les réglages rapides, luminosité, profils d'énergie | D-20 | Le niveau et l'état de charge affichés sont ceux du système |
| J-18 | Énergie : la carte NVIDIA dort hors jeu et se réveille à la demande | J-10, D-16 | Hors jeu, le pilote indique la carte éteinte |
| S-22 | Portable : veille et réveil, couvercle, touches de fonction, Wi-Fi et Bluetooth | S-13 | Dix cycles de veille et réveil sans perdre le Wi-Fi ni l'affichage |
| S-23 | Matériel générique : noyau à pilotes en modules, firmware complet, détection de la carte graphique | S-14 | L'image démarre dans QEMU avec trois cartes graphiques virtuelles différentes |

## Mode jeu et NVIDIA

Le mode NVIDIA impose trois contraintes au compositeur et une précaution sur le pilote.

- **Synchronisation explicite.** NVIDIA la décrit dans ses notes Wayland et demande un noyau 6.8 ou plus récent ([notes Wayland du pilote](https://download.nvidia.com/XFree86/Linux-x86_64/610.43.02/README/wayland-issues.html)). C'est le lot D-15.
- **Modules noyau ouverts.** Ils prennent en charge les cartes Turing et suivantes, soit les RTX 20 et plus récentes ([README des modules ouverts](https://download.nvidia.com/XFree86/Linux-x86_64/555.42.02/README/kernel_open.html)). Cette page date de la série 555 et signalait encore G-Sync absent sur portable. À revérifier sur la version qu'on retiendra.
- **Écran d'un portable hybride.** Les mêmes notes indiquent qu'aucun compositeur Wayland ne gère les multiplexeurs d'affichage. Sur un portable, l'écran interne restera donc piloté par la puce AMD et NVIDIA fera le rendu des jeux. Mesa reste indispensable, y compris pour la CI dans QEMU.
- **Version épinglée.** On fixe une version du pilote validée sur ta machine. On n'en change qu'après le test d'une heure du lot J-11.

**gamescope attendra.** Un ticket chez Valve décrit un plantage sur NVIDIA quand gamescope tourne en fenêtre sous Wayland avec la mise à l'échelle ([ticket 1662](https://github.com/ValveSoftware/gamescope/issues/1662)). Le plein écran passe donc par notre compositeur en 0.1.

**Ce que « optimisé pour NVIDIA » veut dire en 0.1**

- Le jeu en plein écran est affiché sans passer par la composition.
- Fréquence variable et limiteur d'images réglés par jeu.
- Cache de shaders conservé entre les lancements.
- Profil d'énergie performance, notifications coupées, conteneurs et bacs à sable en pause pendant le jeu.
- Des mesures avant et après sur trois jeux (lot J-17), pour ne garder que les réglages qui servent.

**Steam dans notre interface**

- La bibliothèque se lit dans les fichiers de Steam (`libraryfolders.vdf`, puis un `appmanifest` par jeu) et un jeu se lance par l'adresse `steam://rungameid/` ([exemple d'outil qui procède ainsi](https://pypi.org/project/steamcli-launcher/)).
- Le client Steam continue de tourner en arrière-plan, sans fenêtre. Il garde les téléchargements, Proton et les manettes.
- Le catalogue est en lecture seule dans notre interface. L'achat ouvre la page de Steam.
- Ton projet reSteam couvre déjà cette brique. Je propose qu'il devienne l'app Jeux de l'OS, plutôt que d'en écrire une seconde.

## Ton portable et la compatibilité

La machine de référence est un portable hybride : un Ryzen 7 avec sa puce graphique AMD intégrée, et une RTX 3060.

- **La RTX 3060 convient.** C'est une carte de génération Ampere. Les modules ouverts la prennent en charge et sa mise en sommeil hors jeu (RTD3) est prévue à partir de cette génération ([README des modules ouverts](https://download.nvidia.com/XFree86/Linux-x86_64/555.42.02/README/kernel_open.html), [gestion d'énergie RTD3](https://download.nvidia.com/XFree86/Linux-x86_64/575.64/README/dynamicpowermanagement.html)).
- **Deux cartes, deux rôles.** Le compositeur tourne sur la puce AMD, qui pilote l'écran interne. Les jeux sont rendus par la NVIDIA puis affichés par l'AMD. C'est le lot D-16.
- **Écran externe.** Sur beaucoup de portables, la prise HDMI est câblée sur la carte dédiée ([blog de KWin](https://planet.kde.org/xavers-blog-2026-07-31-fixing-multi-gpu-performance-part-1/)). C'est une source connue de lenteurs sous Wayland. À tester tôt sur ta machine.
- **Batterie.** Hors jeu, la NVIDIA doit dormir. C'est le lot J-18.

Pour rendre l'OS compatible avec d'autres machines sans promettre ce qui n'est pas testé, on distingue trois niveaux :

| Niveau | Matériel | Engagement |
| --- | --- | --- |
| 1, référence | Ton portable | Testé à chaque jalon. Rien ne fusionne si ça casse dessus |
| 2, générique | QEMU, puce AMD seule, puce Intel seule, NVIDIA seule à partir des RTX 20 | Construit et testé en machine virtuelle, puis sur du vrai matériel dès qu'une machine est disponible |
| 3, au mieux | Tout le reste | Les pilotes libres sont présents, sans test ni promesse |

Cinq règles en découlent pour tous les lots :

- Le noyau est générique, avec les pilotes courants en modules et le firmware complet.
- Rien n'est écrit pour ta machine en particulier. Tout passe par la détection du matériel.
- L'installeur détecte la carte graphique et propose le bon profil : AMD, Intel, NVIDIA ou hybride.
- Une carte NVIDIA plus ancienne que les RTX 20 reçoit le pilote libre, sans optimisation pour le jeu.
- Le mot « compatible » ne s'écrit que pour ce qui a été testé.

Ces lots ajoutent environ un mois, surtout sur J6 et J7. Je ne décale pas les dates avant que J1 ait donné une vitesse réelle.

## Risques

| Risque | Signe d'alerte | Parade |
| --- | --- | --- |
| Pilote NVIDIA instable sous Wayland | Scintillement, mémoire qui monte, écran figé | Version épinglée, test d'une heure avant toute montée de version, console de secours |
| Périmètre qui gonfle | Un lot sans critère d'acceptation, une idée ajoutée en cours de route | Tout ajout passe par un ADR et repousse un autre lot |
| Code d'agents non relu | Demandes de fusion énormes, tests absents | Lots petits, CI stricte, relecture par un second agent, ton test sur la machine |
| Sherlock agit sans accord | Une action apparaît au journal sans accord associé | Courtier testé automatiquement, lot A-20 avant toute action destructive |
| Maintenance des paquets | Rapport de veille qui s'allonge | Base réduite, veille Q-06, mises à jour groupées une fois par mois |
| Ton temps de relecture | Des demandes de fusion attendent plus d'une semaine | Trois agents en parallèle au maximum |
| Steam change ses fichiers | Bibliothèque vide dans l'app Jeux | Lecture isolée dans un module, repli sur l'ouverture du client Steam |
| Droits sur la mascotte | Le dépôt la présente comme la mascotte de l'assistant d'ITHome | À vérifier avant toute publication de l'OS |

## Questions ouvertes

Aucune ne bloque le jalon J0. Tu as répondu pour la machine : un portable, Ryzen 7 et RTX 3060.

- [ ] Le modèle exact du portable, pour vérifier le câblage de ses prises HDMI et USB-C.
- [ ] D'autres machines disponibles pour tester la compatibilité ?
- [ ] Le nom de l'OS et celui du dépôt.
- [ ] reSteam : dans quelle technologie comptes-tu le développer ?
- [ ] Trois à cinq jeux de référence pour la matrice Proton.
- [ ] Le fournisseur du modèle pour Sherlock.
- [ ] La licence du dépôt, et s'il est public ou privé.

## Sources

- [Notes Wayland du pilote NVIDIA 610.43.02](https://download.nvidia.com/XFree86/Linux-x86_64/610.43.02/README/wayland-issues.html) : synchronisation explicite, noyau requis, multiplexeurs d'affichage.
- [README des modules noyau ouverts, pilote 555.42.02](https://download.nvidia.com/XFree86/Linux-x86_64/555.42.02/README/kernel_open.html) : cartes prises en charge, limites connues.
- [Ticket 1662 de gamescope](https://github.com/ValveSoftware/gamescope/issues/1662) : plantage sur NVIDIA en session Wayland imbriquée.
- [steamcli-launcher](https://pypi.org/project/steamcli-launcher/) : lecture de la bibliothèque Steam et lancement d'un jeu.

* [Gestion d'énergie RTD3, pilote NVIDIA 575.64](https://download.nvidia.com/XFree86/Linux-x86_64/575.64/README/dynamicpowermanagement.html) : mise en sommeil de la carte sur portable.
* [Blog de Xaver Hugl, KWin, 31 juillet 2026](https://planet.kde.org/xavers-blog-2026-07-31-fixing-multi-gpu-performance-part-1/) : écrans externes câblés sur la carte dédiée.

Les durées, le découpage en lots et les critères d'acceptation sont mes propositions, pas des faits sourcés.
