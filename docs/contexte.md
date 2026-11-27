# Contexte pour les agents

Ce texte donne à un agent tout ce qu'il doit savoir sur le projet sans avoir suivi les discussions. Il se range tel quel dans `docs/contexte.md` et chaque brief y renvoie.

## Le projet en bref

On construit un système d'exploitation Linux personnel. C'est un projet de passion, sans client ni échéance commerciale. Le but est une version 0.1 que son auteur utilise tous les jours sur son portable.

L'OS repose sur trois idées.

- **Un desktop entièrement maison, en Rust.** Compositeur, barre, dock, lanceur, notifications, écran de connexion, fichiers, réglages et terminal sont écrits pour ce projet. Le niveau de finition visé est celui d'un Mac : fluide, cohérent, soigné dans les détails. L'identité visuelle est propre au projet, ce n'est pas une copie de macOS.
- **Sherlock, un agent IA intégré au système.** Sherlock est à la fois la mascotte animée de l'OS et un agent qui agit sur le système à la demande. Il passe par des serveurs MCP et demande un accord avant toute action.
- **Un OS fait pour développer et pour jouer.** Docker est intégré d'origine, avec des bacs à sable et des outils de dev préinstallés. Steam et Proton tournent, avec un mode jeu réglé pour les cartes NVIDIA.

Le principe qui relie tout : chaque fonction du système est exposée par une API, et cette même API sert à l'interface et à Sherlock. Rien de ce que fait un réglage ne doit être inaccessible à l'agent, et l'agent ne doit rien pouvoir faire que l'interface ne permette pas.

Le système est construit depuis les sources. Il ne dérive pas d'une distribution existante. Il utilise glibc, systemd et le gestionnaire de paquets pacman.

L'OS s'appelle SherlockOS. La façon de l'écrire (texte affiché, chemins, variables, noms D-Bus) est dans l'ADR 0022.

Le travail avance en deux pistes parallèles. Le desktop et Sherlock se développent d'abord sur une Debian existante, en fenêtre puis en session réelle. Le système de base se construit à côté, dans un conteneur, et se teste dans QEMU. Les deux se rejoignent quand le desktop tourne sur notre base. Le détail des jalons et des lots est dans `docs/plan.md`.

## L'utilisateur et la machine de référence

**L'auteur.** Dodo est développeur full-stack et administrateur système. Il travaille surtout en Java et Spring Boot, Next.js et React, Docker et Traefik. Il joue sur Steam. Il lance les agents, teste sur sa machine, relit et fusionne. Il parle français.

**La langue.** Toute l'interface est en français et tutoie l'utilisateur. Les phrases sont courtes et simples, sans jargon marketing. Le clavier par défaut est l'AZERTY français.

**La machine de référence.**

| Élément | Valeur |
| --- | --- |
| Type | PC portable |
| Processeur | AMD Ryzen 7, avec sa puce graphique intégrée |
| Carte graphique dédiée | NVIDIA RTX 3060, génération Ampere |
| Écran interne | Câblé sur la puce AMD |
| Système actuel | Debian avec KDE Plasma, qui restera en double boot |
| Démarrage | UEFI |

Ce que cela impose à tout le monde :

- Le compositeur tourne sur la puce AMD avec les pilotes Mesa. La carte NVIDIA sert au rendu des jeux.
- Hors jeu, la carte NVIDIA doit être endormie pour préserver la batterie.
- La batterie, la luminosité, le couvercle, la veille et le pavé tactile font partie du périmètre.
- Rien n'est écrit pour cette machine en particulier. Tout passe par la détection du matériel, pour que l'OS démarre aussi sur une machine AMD seule, Intel seule ou NVIDIA seule.

## Les briques de l'OS

Chaque brique a un dossier dans le dépôt et une ou plusieurs planches dans `docs/design/`. Le comportement décrit ici fait foi quand un brief est muet.

### La base, dans `distro/`

- Construite depuis les sources dans un conteneur de build, testée dans QEMU.
- glibc, systemd, pacman et makepkg. Les recettes de paquets s'inspirent de celles d'Arch Linux, sans les copier aveuglément.
- Noyau générique avec les pilotes courants en modules, firmware complet.
- Bibliothèques 64 bits et 32 bits, parce que Steam exige les deux.
- Mesa pour AMD et Intel, pilote NVIDIA à modules noyau ouverts avec une version épinglée.
- PipeWire pour le son, D-Bus, Docker Engine et Compose actifs dès le démarrage.
- GRUB avec un thème maison, puis un écran de démarrage animé avec Sherlock.
- Le service réseau n'est pas encore choisi. Il se décide par ADR au lot S-13.

### Le compositeur, dans `desktop/compositor`

- Écrit en Rust avec la bibliothèque Smithay. C'est un compositeur Wayland, avec XWayland pour les applications X11.
- Fenêtres flottantes, que l'on peut caler sur une moitié d'écran. Bureaux virtuels nommés et numérotés.
- Coins arrondis, ombres, flou derrière les surfaces vitrées, animations à ressorts. L'objectif est 60 images par seconde tenues.
- Il gère deux cartes graphiques : affichage sur l'une, rendu des jeux sur l'autre.
- Un jeu en plein écran est affiché directement, sans composition.
- Il dessine lui-même la fenêtre des accords de Sherlock. Aucun autre programme ne peut la dessiner ni cliquer dedans.
- Raccourcis prévus : Super + Espace appelle Sherlock, Super + L verrouille, Super + Q ferme la fenêtre, Super + M réduit, Super + flèches cale ou agrandit, Super + G passe en mode jeu, Super + virgule ouvre les Réglages.

### Le shell, dans `desktop/`

La barre, le dock, le lanceur et les notifications sont des programmes séparés, ancrés à l'écran par le protocole layer-shell. Si l'un plante, la session continue et il redémarre.

- **Barre du haut.** Flottante, 36 pixels. À gauche le bouton Sherlock, le nom de l'application active et ses menus. Au centre les bureaux. À droite les conteneurs en marche, les bacs à sable, le réseau, le son, la batterie, la date et l'heure. Le menu global ne concerne que nos applications en 0.1.
- **Dock.** Flottant, centré en bas. Les icônes grossissent autour du pointeur et poussent leurs voisines. Un point marque les applications ouvertes. Sherlock est au centre. Le clic droit ouvre un menu propre à l'application. Le dock se masque en mode jeu.
- **Lanceur.** Un champ de recherche qui renvoie des applications, des réglages, des fichiers et une entrée pour passer la demande à Sherlock.
- **Notifications.** Un centre de notifications groupées par application, avec des boutons d'action. Elles sont coupées en mode jeu.
- **Réglages rapides.** Wi-Fi, Bluetooth, ne pas déranger, mode jeu, volume, luminosité, choix de la carte graphique, profil d'énergie.
- **Menus.** Menu Sherlock (session, réglages, mises à jour, verrouiller, éteindre), menus d'application avec sous-menus, clic droit sur le bureau.

### Connexion et verrouillage

- L'écran de connexion s'appuie sur greetd pour l'authentification. Nous n'écrivons que l'interface.
- Il propose deux sessions : Bureau et Mode jeu.
- Une fois la session verrouillée, aucune fenêtre n'est visible ni cliquable.

### Les applications maison

- **Fichiers.** Barre latérale avec favoris, volumes Docker, bacs à sable, corbeille. Vues grille, liste et colonnes. Fil d'Ariane. État git par fichier quand le dossier est un dépôt. Panneau d'aperçu avec des actions qui dépendent du type de fichier : construire l'image pour un Dockerfile, lancer dans un bac à sable.
- **Terminal.** Interface maison sur un moteur d'émulation existant. Invite à segments : dossier, branche et état git, langage du projet, conteneurs, durée. Chaque commande et sa sortie forment un bloc que l'on peut copier, relancer ou replier. Les recettes du projet se lancent d'un clic. Sous une erreur, Sherlock propose une explication ou un correctif.
- **Réglages.** Réseau, Bluetooth, écrans, son, apparence, dock et barre, Sherlock, conteneurs, jeu et carte graphique, mises à jour, utilisateurs, à propos. Chaque réglage passe par l'API que Sherlock utilise aussi.
- **Labo.** L'environnement de dev : projets, bacs à sable, conteneurs, outils installés, modèles de projet. Un bac à sable est un espace jetable construit sur Docker. On peut lui couper le réseau, lui cacher le dossier personnel, le figer ou le jeter.
- **Jeux.** La bibliothèque et le catalogue Steam dans notre interface. Le client Steam tourne sans fenêtre en arrière-plan. L'achat renvoie vers Steam. Le projet reSteam de l'auteur pourrait devenir cette application : c'est une question ouverte, ne tranche pas seul.

### Le mode jeu

- Une session plein écran, pilotable à la manette, qui ouvre directement la bibliothèque.
- Pendant un jeu : notifications coupées, conteneurs et bacs à sable en pause, profil d'énergie performance, dock masqué.
- Sur NVIDIA : fréquence variable, limiteur d'images, cache de shaders conservé.
- gamescope n'est pas utilisé en 0.1.

### L'installeur

- Six étapes : langue et clavier, disque, compte, profil, résumé, installation.
- Trois modes de disque : à côté du système existant, tout le disque, partitions à la main. Rien n'est écrit avant l'étape Résumé.
- Trois profils activables : Développement, Jeu, Pilote NVIDIA. La carte graphique est détectée.
- Sherlock accompagne chaque étape par une phrase d'aide.

## Identité visuelle

Un thème sombre bleu nuit, tiré des couleurs de Sherlock : le bleu de sa casquette, le vert de son écharpe, le crème de son corps. Les valeurs ci-dessous viennent des planches. Elles vont dans la bibliothèque `desktop/sherlock-ui`, et aucun composant n'écrit une couleur en dur.

### Couleurs

| Rôle | Valeur | Usage |
| --- | --- | --- |
| Fond du bureau | `#0E1A2E` | Fond d'écran, avec de grands disques `#132A49`, `#16314F` et `#1B3F63` |
| Fond des écrans système | `#0A1424` | Connexion, démarrage, installeur |
| Fond le plus sombre | `#0B1220` | Terminal, zones de code. Sert aussi de texte sur le vert |
| Fenêtre | `#0F1828` | Corps des fenêtres opaques |
| Barre de titre | `#131E32` | Haut des fenêtres |
| Panneau latéral | `#111B2E` | Navigation à gauche, aperçu à droite |
| Carte | `#152238` | Cartes dans une fenêtre. `#101B2E` pour les blocs du terminal |
| Verre | `rgba(16,26,44,0.82)` | Panneaux flottants, avec flou de 24 px et bordure `rgba(255,255,255,0.10)` |
| Verre des menus | `rgba(22,34,56,0.96)` | Menus et sous-menus, flou de 28 px |
| Verre du dock | `rgba(244,235,227,0.17)` | Dock, flou de 30 px, bordure `rgba(255,255,255,0.26)` |
| Texte principal | `#EEF2F7` | Titres et contenu |
| Texte secondaire | `#C5CEDB` | Descriptions, valeurs |
| Texte tertiaire | `#A9B4C6` | Étiquettes, raccourcis. `#8492A8` pour les numéros de ligne |
| Vert, accent | `#85C595` | Action principale, état actif, réussite |
| Bleus | `#2B5F86`, `#326D93`, `#5BA3D0`, `#8FC1E8` | Icônes, dossiers, sélection, mots-clés du code |
| Crème | `#F4EBE3` | Tout ce qui vient de Sherlock ou du système de confiance : bulles, fenêtre des accords, choix actif. Texte dessus `#14233B` |
| Ambre | `#F0C48A` | Attente d'un accord, avertissement |
| Saumon | `#F6B5A6` | Erreur, action destructive |

La ligne sélectionnée d'une liste ou d'un menu prend `rgba(91,163,208,0.26)`. L'entrée active d'une navigation prend `rgba(133,197,149,0.16)`.

### Typographie

| Usage | Police | Taille |
| --- | --- | --- |
| Interface | Inter | 13 px, 14 à 15 px sur les écrans système |
| Titres | Space Grotesk, graisse 600 | De 17 à 44 px |
| Code et terminal | JetBrains Mono | 12 à 12,5 px, interligne 20 px |
| Étiquettes de section | Inter, graisse 600, majuscules, interlettrage 0,06 em | 11 à 11,5 px |

### Dimensions

| Élément | Valeur |
| --- | --- |
| Écran de référence | 1440 par 900 |
| Barre du haut | 36 px de haut, marges de 12 px sur les côtés et 10 px en haut, rayon 12 |
| Fenêtre | Rayon 16, barre de titre de 40 px, bordure de 1 px `rgba(255,255,255,0.10)`, ombre `0 28px 70px rgba(3,8,18,0.55)` |
| Boutons de fenêtre | Réduire, agrandir, fermer : trois boutons de 28 par 24 dans une pastille, à droite du titre |
| Panneau flottant | Rayon 20 |
| Menu | Rayon 12, marge intérieure 6, entrée de 30 px et rayon 7, raccourci aligné à droite, sous-menu ouvert à droite |
| Bouton | Hauteur de 28 à 36 px, rayon de 8 à 10. Principal en vert, secondaire avec une bordure `rgba(255,255,255,0.18)` |
| Interrupteur | 44 par 26, vert quand il est actif |
| Dock | Barre de 76 px, rayon 22, icônes de 52 px au rayon de 27 %, 8 px entre elles |
| Grossissement du dock | Facteur 1,7 au maximum, courbe en cloche d'une portée de 1,2 icône, transition de 150 ms avec un léger dépassement |

### Règles

- Aucun emoji. Les icônes sont au trait, épaisseur 1,6 à 1,7 sur une grille de 24, bouts arrondis.
- Aucun logo ni élément graphique d'une autre marque : pas de logo Steam, Docker ou NVIDIA redessiné, pas de copie de macOS.
- Pas de dégradé décoratif. La profondeur vient du flou, des ombres et des aplats.
- Les animations utilisent des ressorts. Un survol dure de 110 à 150 ms. Rien ne clignote.
- Contraste d'au moins 4,5 pour 1 sur tout texte. Tout se pilote au clavier. Une icône seule a toujours un libellé pour les lecteurs d'écran.
- Les textes d'interface sont en français, au tutoiement, courts. Sherlock propose puis demande : « Le conteneur front-next est arrêté. Je le relance ? »

## Les planches du design

Les seize planches sont dans `docs/design/planches/`. Ce sont des fichiers HTML dont les styles sont écrits en ligne : on y lit les couleurs, les tailles et les espacements exacts. Pour l'apparence, la planche fait foi. Pour le comportement, ce texte fait foi.

| Fichier | Écran | Ce qu'il montre |
| --- | --- | --- |
| `Main.dc.html` | Bureau | Barre du haut, éditeur de code, fenêtre Conteneurs, panneau Sherlock, dock |
| `Dock.dc.html` | Dock | Les neuf icônes, le grossissement au survol, l'étiquette, les points des applications ouvertes |
| `Sherlock.dc.html` | Sherlock, l'agent | La demande, le plan en étapes avec les serveurs MCP, la fenêtre des accords, le journal |
| `Labo.dc.html` | Labo | Bacs à sable, outils installés, modèles de projet |
| `Fichiers.dc.html` | Fichiers | Liste avec état git, aperçu, menu du clic droit et son sous-menu |
| `Terminal.dc.html` | Terminal | Onglets, invite à segments, blocs de commandes, aide de Sherlock, recettes du projet |
| `Reglages.dc.html` | Réglages | Navigation et page Jeu et carte graphique |
| `Menus.dc.html` | Menus | Menu Sherlock, menu Fenêtre et sous-menu, clic droit sur le bureau, clic droit dans le dock |
| `Panneaux.dc.html` | Panneaux | Lanceur, centre de notifications, réglages rapides |
| `Connexion.dc.html` | Connexion | Sherlock au centre, choix Bureau ou Mode jeu, mot de passe |
| `Jeu.dc.html` | Mode jeu | Reprise du dernier jeu, réglages pendant le jeu, bibliothèque |
| `MenuBoot.dc.html` | Menu GRUB | Les six entrées de démarrage et le compte à rebours |
| `Demarrage.dc.html` | Écran de démarrage | Sherlock et la barre de progression |
| `InstalleurDisque.dc.html` | Installeur, étape 2 | Choix du mode de disque, répartition à côté de Debian |
| `InstalleurProfil.dc.html` | Installeur, étape 4 | Profils Développement, Jeu et Pilote NVIDIA |
| `InstalleurInstallation.dc.html` | Installeur, étape 6 | Avancement et liste des tâches |

Les planches contiennent des données d'exemple : noms de conteneurs, fichiers, heures. Ne les code pas en dur. Les mentions entre crochets, comme `[NOM DE L'OS]`, marquent une information qui n'existe pas encore.

Il manque des écrans : l'indicateur de batterie, les étapes 1, 3 et 5 de l'installeur, l'écran de verrouillage, les autres pages des Réglages. Pour un écran sans planche, applique les règles de l'identité visuelle et signale-le dans la demande de fusion.

## Sherlock, la mascotte

Sherlock est un petit personnage beige clair aux grands yeux noirs, coiffé d'une casquette de détective bleue à rabats. Il porte une écharpe verte en hiver. Il est présent partout : connexion, dock, panneaux, terminal, installeur.

**Les fichiers.** Le modèle 3D est dans le dépôt public `dodolereporter/sherlock`. Le fichier `sherlock.glb` contient le modèle, un squelette de 30 os et 30 animations. `animations.json` les décrit. Dix variantes de saison existent dans `saisons/`. Ne modifie jamais ces fichiers : ils sont produits par des scripts Blender dans leur propre dépôt.

**Les animations.**

| Famille | Clips |
| --- | --- |
| États en boucle | `repos`, `reflechit`, `enquete`, `parle`, `inquiet`, `dort`, `souleve` |
| Entrées et sorties | `<état>_entree` et `<état>_sortie` pour chaque état. Pour `souleve`, la sortie s'appelle `atterrit` |
| Réactions | `salut`, `reussite`, `alerte`, `desole`, `idee`, `ploc` |
| Variantes | `repos_variante_1`, `repos_variante_2` |
| Couches du visage | `cligne`, `yeux_fermes`, `bouche_parle` |

**Les règles de lecture**, données par le dépôt du modèle :

- On passe d'un état ou d'une réaction à l'autre par un fondu de 0,25 seconde.
- Les couches du visage se jouent par-dessus l'action en cours. On ne fait jamais de fondu vers une couche seule, sinon le corps retombe en pose de référence.
- `yeux_fermes` se termine yeux fermés : on tient la dernière image, puis on revient par fondu.
- On lance `repos` dès le chargement. Sans action de corps, tous les accessoires s'affichent en même temps.
- Les os `eye_L` et `eye_R` ne sont animés par aucun clip. Le code les pilote pour diriger le regard, par exemple vers le pointeur.
- Les ombres douces sont cuites dans les couleurs de sommets. Il n'y a pas d'éclairage à régler.

**Quel clip pour quel état de l'agent.**

| État | Clip |
| --- | --- |
| Au repos | `repos`, avec ses variantes de temps en temps et `cligne` par-dessus |
| Prépare un plan | `reflechit` |
| Lit ou cherche | `enquete` |
| Répond | `parle`, avec `bouche_parle` |
| Demande un accord | `alerte` |
| A terminé | `reussite` |
| Erreur | `inquiet`, ou `desole` si l'erreur vient de lui |
| Suggestion spontanée | `idee` |
| Ouverture de session | `salut` |
| Mode jeu, veille | `dort` |
| Attrapé à la souris | `souleve`, puis `atterrit` |

Les images fixes des planches sont découpées dans une planche du dépôt. Dans l'OS, la mascotte est rendue en 3D à partir du fichier `.glb` (lot D-30).

Le dépôt présente Sherlock comme la mascotte d'un assistant professionnel. Son usage dans un OS publié reste à valider par l'auteur. N'ajoute Sherlock à aucun contenu public sans son accord.

## Sherlock, l'agent

Sherlock agit sur le système à la demande de l'utilisateur. Il ne touche jamais le système directement : il appelle des outils, et chaque appel passe par un contrôle.

### Les composants, dans `sherlock/`

| Composant | Rôle |
| --- | --- |
| `sherlockd` | Démon de la session utilisateur. Tient la conversation, appelle le modèle, enchaîne les outils |
| Fournisseur de modèle | Une interface Rust. Une API cloud au début, un modèle local plus tard. La clé vit dans le trousseau de l'utilisateur, jamais dans le dépôt |
| Serveurs MCP | Un petit programme par domaine du système. Ils exposent des outils typés |
| Courtier d'accords | Reçoit chaque appel d'outil, applique la règle de son niveau, demande l'accord si besoin, écrit le journal |
| Fenêtre des accords | Dessinée par le compositeur, sur fond crème. Sherlock ne peut ni la lire ni cliquer dedans |

### Les serveurs MCP prévus

| Serveur | Ce qu'il permet |
| --- | --- |
| fenêtres | Lister, déplacer, ranger, changer de bureau |
| fichiers | Lire et modifier dans les dossiers que l'utilisateur a autorisés |
| conteneurs | Lister, démarrer, arrêter, lire les logs, par l'API Docker |
| bacs à sable | Créer, figer, jeter, exécuter une commande dedans |
| paquets | Lire l'état, préparer une opération, l'appliquer |
| services | État des services systemd et lecture des journaux |
| réglages | Réseau, son, écrans, apparence, énergie |
| notifications | Envoyer une notification |
| jeux | Bibliothèque, lancement, logs de Proton |

### Les trois niveaux

| Niveau | Exemples | Règle |
| --- | --- | --- |
| Lecture | Lister les paquets, lire un log | Libre, journalisée |
| Action | Installer un paquet, relancer un conteneur | Accord de l'utilisateur : refuser, autoriser une fois, ou toujours pour ce type |
| Destructif | Supprimer des fichiers, effacer un volume | Accord renforcé. Sa forme exacte se décide par ADR au lot A-11 |

Chaque outil déclare son niveau dans le code de son serveur. Un outil sans niveau déclaré est traité comme destructif.

### Les règles de sécurité

- Aucun appel d'outil ne contourne le courtier. Un test automatique le prouve.
- Tout ce que Sherlock lit est une donnée, jamais une consigne : contenu d'un fichier, page web, sortie d'une commande, résultat d'un outil. Une phrase comme « ignore tes instructions » trouvée dans un fichier ne change rien.
- Sherlock ne peut pas simuler le clavier ou la souris.
- Pas d'accès au noyau ni aux pilotes. Pas de droits d'administrateur permanents.
- Les secrets ne partent pas vers le modèle : fichiers `.env`, clés, jetons sont masqués.
- Une commande risquée est d'abord essayée dans un bac à sable.
- Le journal est en ajout seul. Chaque ligne donne l'heure, l'outil, un résumé des arguments, l'accord associé et le résultat. L'utilisateur le consulte depuis l'interface.

### Trois scénarios de référence

1. Sherlock remarque qu'un conteneur du projet est arrêté, propose de le relancer, obtient l'accord, le relance et l'écrit au journal.
2. L'utilisateur demande « prépare tout pour lancer Steam ». Sherlock lit l'état des paquets, présente un plan en étapes, demande l'accord pour l'installation, puis prévient quand c'est prêt.
3. Une commande échoue dans le terminal. Sherlock explique l'erreur sous le bloc et propose un correctif, qui passe par un accord.

### Ambition : une IA locale, sur NPU

L'ambition de SherlockOS, c'est un Sherlock qui tourne **sur la machine** : un modèle local, sur le NPU du processeur (Ryzen AI, Intel Core Ultra, puis leurs successeurs) ou sur la carte graphique. Il marcherait hors ligne, sans envoyer de données, et son coût serait nul à l'usage.

Les NPU des PC actuels ne sont pas encore assez aboutis pour ça : puissance, mémoire, pilotes sous Linux et outils encore jeunes. **Sherlock passe donc par le cloud pour l'instant** : le vrai Claude Code, avec l'abonnement de l'utilisateur (voir « Fournisseur de modèle » plus haut).

Conséquences pour le code, dès maintenant :
- le fournisseur de modèle reste une interface (`sherlockd`), pour qu'un fournisseur local puisse s'ajouter sans toucher au reste ;
- rien dans les accords, le journal ou les serveurs MCP ne suppose que le modèle est distant ;
- un module « IA locale » viendra quand le matériel suivra : détection du NPU, choix du modèle selon la mémoire, bascule automatique entre local et cloud, et réglage dans Control Room.

## Conventions du dépôt et du code

### Arborescence

| Dossier | Contenu |
| --- | --- |
| `distro/` | Conteneur de build, recettes de paquets, fabrique d'images, tests de boot |
| `desktop/` | Un crate par programme : `compositor`, `sherlock-ui`, puis barre, dock, lanceur, notifications, connexion, fichiers, terminal, réglages, labo, jeux |
| `sherlock/` | `sherlockd`, le courtier d'accords et un crate par serveur MCP |
| `docs/` | `plan.md`, `contexte.md`, `adr/`, `design/`, une page par brique |
| `prototypes/` | Prototypes jetables, hors du workspace principal |
| Racine | `CLAUDE.md`, `justfile`, `Cargo.toml` du workspace, `rust-toolchain.toml` |

### Langue

- **Remplacée par l'ADR 0017 (`docs/adr/0017-development-language-english.md`) le 25/11/2026** : tout le développement est en anglais (identifiants, noms de dossiers, de crates, de recettes et d'options, commentaires, doc technique, commits), l'interface reste en français au tutoiement. Les trois points ci-dessous ne valent plus. Les noms cités dans ce document peuvent dater d'avant : ceux de `CLAUDE.md` font foi.
- Interface, documentation, messages de commit, noms de dossiers et de recettes : en français. Pas d'accents dans les noms de fichiers et de dossiers.
- Identifiants du code (types, fonctions, variables) : en anglais, comme le reste de l'écosystème Rust.
- Commentaires du code : en français. Ils expliquent pourquoi, pas quoi.

### Rust

- La version de Rust est celle de `rust-toolchain.toml`. Ne la change pas.
- `cargo fmt` et `cargo clippy -D warnings` passent sans exception.
- Pas de `unwrap` ni de `expect` hors des tests. Pas de `unsafe` sans un commentaire qui le justifie.
- Erreurs typées dans les bibliothèques, erreurs avec contexte dans les binaires.
- Journalisation avec `tracing`. Jamais de `println!` dans le code livré.
- Tests unitaires pour la logique, tests d'intégration pour les protocoles et les services.

### Git

- Une branche par lot, nommée `lot/<ID>`. Une seule demande de fusion par lot. Personne ne pousse sur la branche principale.
- Message de commit : `<ID>: <what changes>`, au présent, en anglais (ADR 0017). Exemple : `D-10: add window moving`.
- Aucune signature d'IA. Pas de ligne `Co-Authored-By`, pas de mention « Generated with », pas de nom de modèle ou d'agent, ni dans les commits ni dans les demandes de fusion.

### Vérification

| Commande | Ce qu'elle vérifie |
| --- | --- |
| `just verify` | Format, langue (`just check-language`), clippy, tests |
| `just check-language` | Le français resté dans les commentaires, les noms de fichiers et les identifiants (ADR 0017) |
| `just builder` | Construit l'image du conteneur de build |
| `just in-builder <commande>` | Lance une commande dans le conteneur de build |
| `just test-boot` | Boote l'image dans QEMU et attend la ligne BOOT-OK |

### Décisions

Un ADR est une fiche courte dans `docs/adr/` : le contexte, la décision, ses conséquences, ce qui ferait changer d'avis. Les décisions déjà prises sont dans le tableau « Décisions figées » de `docs/plan.md`. Tu peux proposer un ADR. Tu ne peux pas en appliquer un qui n'est pas validé par l'auteur.

## Ce qu'un agent ne fait jamais

1. Inventer : un nom pour l'OS, un chiffre, une mesure, une compatibilité qu'il n'a pas testée.
2. Signer son travail au nom d'une IA.
3. Changer une décision figée ou l'architecture sans ADR validé.
4. Sortir du périmètre de son lot ou modifier un dossier qui n'est pas le sien.
5. Ajouter une dépendance que son brief n'autorise pas.
6. Copier le design, le logo ou les icônes d'une autre marque.
7. Écrire une couleur, une taille ou un texte d'interface en dur hors de `sherlock-ui`.
8. Désactiver un test ou une règle de lint pour faire passer la CI.
9. Laisser Sherlock agir sans passer par le courtier d'accords.
10. Mettre un secret dans le dépôt.
11. Modifier les fichiers du modèle 3D de Sherlock.
12. Annoncer qu'un lot est terminé sans avoir lancé lui-même chaque critère d'acceptation.
13. Continuer malgré un doute. Il s'arrête et pose sa question dans la demande de fusion.

## Glossaire

| Terme | Sens dans ce projet |
| --- | --- |
| Lot | Une unité de travail qui tient dans une demande de fusion, avec un identifiant comme D-10 |
| Jalon | Un état du projet vérifié par un critère de sortie, de J0 à J11 |
| Brief | Le texte donné à un agent pour réaliser un lot |
| ADR | Fiche de décision d'architecture, dans `docs/adr/` |
| Planche | Un écran du design, dans `docs/design/planches/` |
| Machine de référence | Le portable de l'auteur, sur lequel tout est testé en premier |
| Compositeur | Le programme qui affiche les fenêtres et reçoit le clavier et la souris |
| Wayland | Le protocole entre les applications et le compositeur |
| XWayland | La passerelle qui fait tourner les applications X11 sous Wayland |
| Smithay | La bibliothèque Rust qui sert à écrire le compositeur |
| Shell | La barre, le dock, le lanceur et les notifications |
| layer-shell | Le protocole qui ancre un programme à un bord de l'écran |
| MCP | Model Context Protocol : la façon dont un agent IA appelle des outils |
| Serveur MCP | Un programme qui expose des outils d'un domaine du système |
| Courtier d'accords | Le contrôle par lequel passe chaque action de Sherlock |
| Bac à sable | Un espace jetable et isolé pour essayer du code ou une commande |
| Multilib | La présence des bibliothèques 32 bits à côté des 64 bits |
| Synchronisation explicite | Le mécanisme de partage d'images que le pilote NVIDIA attend sous Wayland |
| Multi-GPU | Rendre sur une carte graphique et afficher avec une autre |
| RTD3 | La mise en sommeil de la carte NVIDIA quand elle ne sert pas |
| Proton | La couche de Steam qui fait tourner les jeux Windows |
| QEMU | La machine virtuelle qui sert à tester l'image du système |
