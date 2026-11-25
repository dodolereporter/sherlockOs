# Compositeur

Le compositeur affiche les fenêtres des applications Wayland et reçoit le clavier et la souris.
Il est écrit en Rust avec Smithay 0.7, dans `desktop/compositeur`. Le lot D-10 en pose la base :
il tourne en fenêtre dans une session existante, sans effet visuel. Le lot D-17 rend les fenêtres
faciles à prendre en main : bande de saisie, curseurs, calage contre les bords de l'écran. Le lot
D-11 lui donne l'apparence des planches : fond du bureau, coins arrondis, bordure, ombre, et
animations à ressorts. Le lot D-13 y fait tourner les applications X11, par XWayland, avec le
presse-papiers dans les deux sens.

Prérequis du poste, en plus de ceux de `CLAUDE.md` : le serveur `Xwayland` (paquet `xwayland`
sur Debian). Essayé avec la version 24.1.6 de Debian 13 ; la CI installe celle d'Ubuntu 24.04. Sans
lui, le compositeur démarre quand même, et seules les applications Wayland s'affichent.

## Architecture

| Fichier | Rôle |
| --- | --- |
| `src/main.rs` | Lit les options (`--imbrique`, `--mesurer`), crée la boucle d'événements, démarre XWayland, lance la commande donnée |
| `src/lib.rs` | `State` : affichage Wayland, protocoles, siège, `Space` des fenêtres. Socket, écran, focus |
| `src/handlers.rs` | Réponses aux protocoles : `wl_compositor`, `wl_shm`, `wl_seat`, `wl_output`, xdg-shell, formes de curseur, presse-papiers et sélection primaire. Pose d'une fenêtre selon son état (`rearrange`), qu'elle soit Wayland ou X11 |
| `src/xwayland.rs` | XWayland : démarrage et arrêt, gestionnaire de fenêtres X11 (`X11Wm`), association des surfaces, presse-papiers vu de X11 |
| `src/focus.rs` | Cible du clavier : surface Wayland ou fenêtre X11 |
| `src/input.rs` | Clavier et souris : focus au clic, gestes, calage, raccourcis, forme du curseur |
| `src/layout.rs` | Disposition : placer, déplacer, redimensionner, bande de saisie, zones de calage, retour à la taille d'avant. Du calcul pur, sans Wayland ni rendu |
| `src/winit.rs` | Backend fenêtré : la fenêtre hôte, son écran virtuel, son curseur. Lance le rendu de chaque image et sa mesure |
| `src/render.rs` | Rendu : fond et disques, ombres, contenu découpé en rectangle arrondi, bordures, aperçu du calage |
| `src/shaders/` | Les trois programmes GLSL du rendu : `shape.frag`, `shadow.frag`, `clip.frag` |
| `src/anim.rs` | Mouvements des fenêtres et de l'aperçu : ressorts appliqués aux rectangles. Du calcul pur |
| `src/stats.rs` | Statistiques des images pour `--mesurer` |
| `examples/scene.rs` | Client de test : la scène de mesure, et une fenêtre pour comparer à la planche |

Les couleurs, les tailles, les durées et les ressorts viennent de `sherlock-ui` (modules `color`,
`metrics`, `motion` et `window`), sans sa fonction `iced`.

Tout tourne dans un seul fil, sur une boucle `calloop`. Les sources de la boucle sont l'affichage
Wayland (requêtes des clients), la socket d'écoute et, en mode imbriqué, la fenêtre winit
(entrées et demandes de dessin). Après chaque tour, `State::refresh` range les fenêtres mortes
et envoie les réponses aux clients.

La logique de disposition vit dans `layout.rs` et ne connaît que des rectangles : le rendu
(`winit.rs`) et les entrées (`input.rs`) l'appellent. Le futur backend DRM (lot D-14) réutilisera
`State` tel quel et ne remplacera que `winit.rs`.

Comportement :

- Une nouvelle fenêtre est centrée sur l'écran à son premier contenu et reçoit le clavier.
- Un clic sur une fenêtre lui donne le clavier et la passe au premier plan.
- La touche des gestes est Super en session réelle (lot D-14), et Alt en mode imbriqué
  (`--imbrique`) : la session hôte garde Super pour elle (sous Plasma, les actions des fenêtres
  de KWin et le lanceur). C'est la convention de niri. La règle tient dans
  `gesture_modifier_held` (`src/input.rs`).
- Les fenêtres dessinent leur barre de titre elles-mêmes : xdg-decoration est hors du lot.
- Le clavier est en AZERTY français, sauf si `XKB_DEFAULT_LAYOUT` est défini.

Déplacer et redimensionner :

- Une bande de 8 pixels entoure chaque fenêtre, à l'extérieur de sa géométrie. Elle s'élargit à
  20 pixels aux coins, en diagonale comme le long des côtés. Un appui du bouton gauche dans la
  bande tire ce bord ou ce coin. Le client ne voit pas l'appui. La bande d'une fenêtre passe
  devant les fenêtres qu'elle recouvre, pas devant celles du dessus. Une fenêtre sans contenu
  n'a pas de bande.
- Glisser la barre de titre ou un bord qu'un client dessine lui-même déplace ou redimensionne sa
  fenêtre (`xdg_toplevel.move` et `.resize`). Comme le veut xdg-shell, la demande n'est accordée
  que sur le siège du compositeur, pendant l'appui d'un bouton sur une surface de ce client, avec
  le serial de cet appui.
- Touche des gestes + glisser avec le bouton gauche déplace la fenêtre sous le pointeur. Avec le
  bouton droit, elle est redimensionnée par le coin le plus proche du pointeur.
- Pendant un redimensionnement, le coin opposé reste en place, et les tailles minimale et
  maximale du client sont respectées.
- Un déplacement ne bouge la fenêtre qu'une fois que le pointeur a parcouru 4 pixels : un simple
  clic ne la décale pas et ne la détache pas de sa zone.
- Seul le relâchement du bouton qui a lancé le geste le termine. Les autres boutons vont au
  siège, sans client qui les reçoive.
- Une fenêtre en plein écran ne se déplace ni ne se redimensionne par un geste, et n'a pas de
  bande.

Caler :

- Une fenêtre déplacée, lâchée avec le pointeur à moins de 8 pixels du bord haut de l'écran, est
  agrandie. Contre le bord gauche ou droit, elle prend cette moitié de l'écran. Dans un coin, à
  moins de 64 pixels du coin le long d'un bord, elle prend ce quart. Le bord bas seul ne cale
  rien. Pendant le glissement, un rectangle vert translucide montre la zone visée.
- Cela vaut pour les déplacements à la touche des gestes comme pour ceux demandés par le client.
- Déplacer une fenêtre calée ou agrandie lui rend sa taille d'avant, sous le pointeur, saisie à
  la même proportion de sa largeur. Si elle n'en avait pas, parce qu'elle a été agrandie avant
  son premier contenu, le client choisit sa taille. La redimensionner lui retire l'état agrandi
  ou calé, et elle part de sa taille actuelle.
- Une fenêtre calée au lâcher retrouvera, en quittant sa zone, sa place d'avant le glisser.
- Touche des gestes + flèche haut agrandit la fenêtre qui a le clavier, ou lui rend sa taille
  d'avant si elle l'est déjà. Flèche gauche ou droite : moitié de l'écran. Flèche bas : taille
  d'avant. Seule la touche des gestes doit être tenue : avec Maj ou Ctrl en plus, la flèche va
  au client. Ces touches, appui comme relâchement, ne vont pas au client. Elles sont sans effet
  en plein écran. Si la fenêtre hôte perd le clavier entre l'appui et le relâchement, le
  compositeur oublie l'interception.
- L'agrandissement est l'état `maximized` de xdg-shell, que le client connaît. Une moitié ou un
  quart n'est qu'une taille imposée : les états `tiled` ne sont pas envoyés.

Curseurs :

- Au-dessus de la bande, le curseur prend la forme du bord ou du coin. Pendant un déplacement,
  il prend la forme « déplacer », et pendant un redimensionnement celle du bord tiré.
- Au-dessus d'un client, le curseur est celui qu'il nomme par le protocole des formes de curseur
  (`wp_cursor_shape_v1`, que Qt 6 utilise). Un client qui dessine lui-même l'image de son
  curseur (`wl_pointer.set_cursor` avec une surface) a la flèche : l'image n'est pas affichée.
  Ailleurs, c'est la flèche.
- En mode imbriqué, c'est le curseur de la fenêtre hôte qui change de forme, avec le thème de la
  session hôte. La session réelle (lot D-14) devra dessiner le curseur elle-même.

## XWayland

XWayland est un serveur X11 qui est aussi un client Wayland du compositeur. Chaque fenêtre X11
y a une surface Wayland, que le protocole xwayland-shell associe à la fenêtre. Smithay fournit le
lancement (`XWayland`) et le gestionnaire de fenêtres X11 (`X11Wm`), qui parle au serveur X au
nom du compositeur. Les fenêtres X11 entrent dans le même `Space` que les fenêtres Wayland, sous
forme de `Window` : la disposition, les gestes, le rendu et les animations ne font pas la
différence, sauf aux quelques endroits où un état se lit ou s'envoie.

**Démarrage au lancement.** XWayland démarre avec le compositeur, pas à la première connexion
d'un client X11. Raisons :

- La commande lancée par `--imbrique` doit connaître `DISPLAY`, et pouvoir être elle-même une
  application X11 : elle part une fois XWayland prêt, avec `DISPLAY=:N` et `WAYLAND_DISPLAY`.
- Smithay 0.7 ne sait que lancer XWayland tout de suite. Le démarrage à la demande demanderait
  d'écouter nous-mêmes les sockets X11 et de passer la première connexion au serveur : plus de
  code, pour un gain faible.
- Le coût est modeste : sur le portable de référence, XWayland est prêt en 0,2 s et occupe
  environ 85 Mo de mémoire résidente (bibliothèques partagées comprises), sans client X11.

Le numéro d'affichage est le premier libre (`:1` sous une session X11 qui occupe `:0`). Il est
dans le journal : `XWayland prêt display=1`. Les messages de Xwayland vont sur la sortie d'erreur
du compositeur, mêlés à ses journaux. Au démarrage, les avertissements de glamor, de GBM et de
xkbcomp sont attendus : sans accès à la puce graphique, Xwayland dessine en logiciel. À l'arrêt,
`(EE) failed to read Wayland events` aussi : le compositeur ferme sa connexion. Sans `Xwayland` sur le poste, le journal le dit
(`pas d'applications X11`), et la commande part sans `DISPLAY` : elle n'hérite pas de celui de
la session hôte, où ses fenêtres X11 s'ouvriraient hors du compositeur.

**Arrêt.** En quittant, le compositeur retire de la boucle XWayland et sa socket Wayland
(`State::shutdown`) : le serveur X s'arrête, son verrou (`/tmp/.X<N>-lock`) et sa socket
(`/tmp/.X11-unix/X<N>`) disparaissent, et la socket Wayland (`$XDG_RUNTIME_DIR/wayland-N`) et son
verrou aussi. Il faut le faire à la main : la source de `X11Wm` garde une poignée de la boucle,
qui n'est donc jamais libérée, ni rien de ce qu'elle contient. Si `X11Wm` ne démarre pas, le
compositeur coupe tout de suite la connexion de XWayland et l'arrête : les applications X11 ne
s'affichent pas, mais la session continue. Tué par un signal (Ctrl + C dans le terminal), le compositeur laisse
le verrou ; Smithay le reprend au lancement suivant, puisque son processus n'existe plus. Le
compositeur crée `/tmp/.X11-unix` s'il manque, avec les droits que lui donne systemd (1777).

**Fenêtres gérées.**

- Une fenêtre qui demande à s'afficher est centrée sur l'écran, à la taille qu'elle demande. Elle
  se montre (avec l'animation d'ouverture) et reçoit le clavier quand XWayland lui associe sa
  surface : avant, il n'aurait pas reçu l'entrée du clavier.
- Le clavier passe par la surface Wayland et par le focus X11 (`SetInputFocus` ou
  `WM_TAKE_FOCUS`, selon ce que la fenêtre accepte). Une fenêtre qui passe au premier plan y
  passe aussi pour le serveur X.
- Le compositeur pose la fenêtre : il envoie au serveur X sa place et sa taille à chaque
  déplacement, calage ou redimensionnement. Une fenêtre flottante obtient la taille qu'elle
  demande, jamais la place. Agrandie, calée ou en plein écran, elle garde celle qu'on lui a
  donnée.
- Les états agrandi et plein écran sont ceux de `_NET_WM_STATE`, que la fenêtre connaît, comme
  ceux de xdg-shell pour une fenêtre Wayland. Ses demandes (`_NET_WM_STATE` et
  `_NET_WM_MOVERESIZE`, par exemple la barre de titre d'un navigateur) passent par les mêmes
  chemins que celles d'une fenêtre Wayland. Comme le serveur X ne donne pas de serial, un geste
  demandé n'est accordé que pendant l'appui d'un bouton sur la surface de cette fenêtre même.
  Une surface de XWayland ne suffit pas : tous les clients X11 sont un seul client Wayland, et
  un autre programme X11 pourrait sinon déplacer sa fenêtre pendant un clic dans xterm.
- Une fenêtre X11 prend sa nouvelle taille dès qu'on la lui envoie. Pendant un
  redimensionnement, sa place est donc recalée tout de suite, sans attendre son contenu.
- Le titre est `_NET_WM_NAME` ou `WM_NAME`.

**Menus et bulles.** Les fenêtres override-redirect se placent elles-mêmes, au-dessus des autres.
Elles sont dessinées telles quelles, comme les menus Wayland : sans coins, ni bordure, ni ombre,
ni animation, ni bande de saisie. Un clic dedans ne change pas le clavier de place.

**Presse-papiers.** Le compositeur annonce `wl_data_device_manager` (CLIPBOARD) et la sélection
primaire (`zwp_primary_selection_device_manager_v1`, PRIMARY). Le client qui a le clavier lit et
pose les deux. Les échanges avec X11 passent par `X11Wm` :

- Un client Wayland pose une sélection : le gestionnaire de fenêtres la prend côté X. Un client
  X11 qui la lit reçoit les données du client Wayland.
- Un client X11 pose une sélection : le compositeur la pose côté Wayland pour son compte. Un
  client Wayland qui la lit reçoit les données du client X11.
- Comme un client Wayland, un client X11 ne lit une sélection venue de Wayland, et ne pose la
  sienne côté Wayland, que si une fenêtre X11 a le clavier. Un programme X11 en arrière-plan ne
  peut donc pas changer ce que l'utilisateur colle dans une application Wayland. La règle est
  stricte, sans exception pour les outils en ligne de commande : `xclip -i` lancé depuis une
  Konsole (Wayland) ne pose rien côté Wayland. Le journal le dit au niveau `debug` :
  `sélection X11 refusée`.


Chaque image est redessinée en entier avec le moteur GLES de Smithay (`GlesRenderer`), dans le
seul fil du compositeur. `render::Scene` compile trois programmes au démarrage et pose les
éléments de l'avant vers l'arrière, comme Smithay les attend :

1. l'aperçu du calage : rectangle arrondi vert translucide, bordure verte de 2 px ;
2. pour chaque fenêtre, de la plus haute à la plus basse : ses menus (`xdg_popup`, ni découpés
   ni mis à l'échelle), sa bordure, son contenu découpé, son ombre ;
3. les trois disques du fond ;
4. la couleur du bureau, qui remplit le reste.

| Programme | Sorte | Ce qu'il dessine |
| --- | --- | --- |
| `shape.frag` | Pixels | Rectangle arrondi rempli et bordé, lissé sur un pixel : bordure des fenêtres (sans remplissage), disques du fond (rayon de la moitié du côté), aperçu du calage |
| `shadow.frag` | Pixels | Ombre portée d'un rectangle arrondi flouté par une gaussienne, effacée sous la fenêtre |
| `clip.frag` | Texture | Contenu du client multiplié par la couverture du rectangle arrondi de sa fenêtre |

Les valeurs, toutes tirées de `sherlock-ui` :

| Élément | Valeur | Jeton |
| --- | --- | --- |
| Fond | `#0E1A2E` | `color::DESKTOP` |
| Disques | `#132A49`, `#16314F`, `#1B3F63`, places et diamètres de la planche Main | `color::DESKTOP_DISCS`, `metrics::DESKTOP_DISC_GEOMETRY` |
| Coins | Rayon 16, réduit pour une fenêtre plus petite que deux coins | `metrics::WINDOW_RADIUS` |
| Bordure | 1 px `rgba(255,255,255,0.10)`, à l'intérieur de la fenêtre comme en CSS | `metrics::BORDER_WIDTH`, `color::BORDER` |
| Ombre | `0 28px 70px rgba(3,8,18,0.55)` | `metrics::SHADOW_OFFSET`, `metrics::SHADOW_BLUR`, `color::SHADOW` |

**Fond.** Sur un écran de 1440 par 900, les disques sont exactement à leur place de la planche :
(-260, 340) diamètre 1100, (860, -420) diamètre 900, (980, 520) diamètre 520, le dernier devant.
Sur un autre écran, leurs centres suivent l'écran sur chaque axe et leur diamètre suit le plus
grand des deux rapports, pour qu'ils restent ronds.

**Coins.** Le contenu du client est vraiment découpé : `RoundedSurface` enveloppe chaque surface
de la fenêtre et remplace, le temps de la dessiner, le programme de texture de Smithay par
`clip.frag`. La découpe se calcule sur `gl_FragCoord`, en pixels de l'image : elle vaut pour les
sous-surfaces comme pour la surface principale, à n'importe quelle échelle. Le backend fenêtré
retourne l'image (`Flipped180`) ; `clip_in_frame` applique la même transformation au rectangle de
découpe. Ce que le client dessine hors de sa géométrie (ombres côté client de GTK) est coupé :
c'est le compositeur qui dessine l'ombre. Une fenêtre en plein écran n'a ni coins, ni bordure,
ni ombre : elle les garde pendant qu'elle glisse vers l'écran entier et les perd à l'arrivée,
quand elle couvre déjà l'écran. Une fenêtre agrandie ou calée les garde.

**Ombre.** Comme `box-shadow` : le rectangle de la fenêtre, décalé de 28 vers le bas, flouté par
une gaussienne d'écart type 35 (la moitié du flou, comme le font les navigateurs). L'élément
s'étend de 70 autour de ce rectangle. Le calcul suit Evan Wallace (« Fast Rounded Rectangle
Shadows ») : intégrale exacte en x, quatre échantillons en y. L'ombre n'est pas dessinée sous la
fenêtre, ce qui compte pour un client translucide et pendant les fondus.

**Coût.** Les surfaces découpées n'ont pas de région opaque : les fenêtres du dessous sont
toujours dessinées. L'image est redessinée en entier à chaque fois (âge 0), comme avant ce lot.
Le backend DRM (D-14) pourra rendre aux surfaces leur partie opaque, hors des coins.

### Animations

Le module `sherlock_ui::motion` porte la physique, en calcul pur et sans iced, pour que le
compositeur et le shell partagent les mêmes ressorts :

- `Spring` : ressort amorti de masse 1, défini par sa raideur et son amortissement. `solve` donne
  l'écart et la vitesse à n'importe quel instant par la solution exacte de l'équation (amorti
  sous-critique, critique ou sur-critique), sans pas de temps : une image en retard ne change
  pas la trajectoire.
- `Animation` : une valeur menée par un ressort. `retarget` change de cible en gardant valeur et
  vitesse : rien ne saute quand la cible bouge en route.
- `Fade` : un fondu sur une durée fixe, départ vif et arrivée douce (cubique). Interrompu, il
  repart de sa valeur.

| Jeton | Valeur | Taux d'amortissement | Usage |
| --- | --- | --- | --- |
| `SNAP_SPRING` | raideur 500, amortissement 33,5 | 0,75, dépassement de 3 % | Calage, agrandissement, retour |
| `OPEN_SPRING` | raideur 450, amortissement 34 | 0,80, dépassement de 2 % | Échelle à l'ouverture, de `OPEN_SCALE` (0,92) à 1 |
| `PREVIEW_SPRING` | raideur 700, amortissement 42 | 0,79 | Aperçu du calage |
| `FADE` | 150 ms | | Opacité à l'ouverture, apparition et effacement de l'aperçu |

`docs/contexte.md` ne donne que « ressorts », « léger dépassement » et des survols de 110 à
150 ms : ces valeurs sont des choix du lot. Chaque ressort fait 90 % du trajet en moins de
150 ms et se pose à 1 % près en moins de 300 ms (tests de `motion`). Le fondu dure un survol.

`anim.rs` les applique aux fenêtres :

- **Ouverture.** À sa première image, la fenêtre grandit de 0,92 à 1 autour de son centre et
  passe de transparente à opaque. Son ombre et sa bordure suivent.
- **Calage et retour.** `rearrange` (calage, agrandissement, plein écran, retour) appelle
  `animate_next_change`. Le drapeau ne vaut que pour l'image suivante, où il est consommé dans
  tous les cas : si la géométrie y a changé, elle part de la dernière géométrie dessinée sur
  quatre ressorts, position et taille ; sinon rien ne s'anime. Une animation déjà en cours
  change simplement de cible. La cible suit ensuite la géométrie réelle : la position change
  tout de suite, la taille quand le client répond, et le ressort reprend sa course sans
  à-coup. Pendant l'animation, le tampon du client est étiré dans le rectangle animé
  (`RescaleRenderElement`). Une fois posé, il est dessiné à l'échelle 1. Limite : un calage qui
  ne change que la taille, la position restant la même, n'est pas animé.
- **Glisser et redimensionner.** Ils ne sont jamais animés : le début d'un geste arrête toute
  animation de géométrie (`WindowMotion::follow`), et la fenêtre suit le pointeur sans délai.
  Une fenêtre calée ou agrandie qu'on détache en la glissant reprend sa taille d'avant d'un
  coup, sous le pointeur, comme avant ce lot.
- **Aperçu du calage.** Il apparaît en grandissant de 0,92 à 1 et en fondu, glisse d'une zone à
  l'autre, et s'efface à sa dernière place.

Rien ne clignote : une fenêtre apparaît depuis une opacité nulle, et chaque animation part de ce
qui était dessiné à l'image d'avant.

### Ce que le rendu ne fait pas encore

- **Flou** derrière les surfaces vitrées (dual Kawase, rayons de 24, 28 et 30 px) : reporté,
  le lot dépassait déjà sa taille. Il demande un rendu hors écran de ce qui est derrière la
  surface, à résolution réduite, puis des passes de flou ; l'aperçu du calage en serait le
  premier client.
- **Fermeture animée.** Quand le client détruit sa fenêtre, son tampon disparaît avec elle.
  L'animer demande de garder une copie de sa dernière image (texture) au moment de la
  destruction.
- Les **survols** : le compositeur n'a encore rien à survoler. Le jeton `motion::FADE` est déjà
  à la durée d'un survol.

## Lancer

```sh
cargo run -p compositeur -- --imbrique
```

Une fenêtre s'ouvre dans la session en cours, Wayland ou X11. Tout ce qui suit `--imbrique` est une
commande, lancée une fois XWayland prêt, avec la socket du compositeur dans `WAYLAND_DISPLAY` et
son affichage X11 dans `DISPLAY`. Ce que cette commande lance à son tour hérite des deux. Pour un
terminal :

```sh
cargo run -p compositeur -- --imbrique env QT_QPA_PLATFORM=wayland konsole --separate
```

Les journaux s'affichent sur la sortie d'erreur, au niveau `info` par défaut. `RUST_LOG` le règle,
par exemple `RUST_LOG=compositeur=debug,smithay=warn`. Le nom de la socket apparaît au démarrage.
Au niveau `debug`, chaque geste écrit son début et sa fin, avec son type (bords tirés, ou aucun
pour un déplacement) et sa source (touche des gestes ou demande du client). Une demande du client
refusée l'est aussi.

`QT_QPA_PLATFORM=wayland` force Konsole à parler Wayland : dans une session X11, Qt choisirait X11,
et Konsole passerait par XWayland. `--separate` évite que la commande rejoigne une Konsole déjà
ouverte.

### Une application X11

```sh
cargo run -p compositeur -- --imbrique xterm
```

Ou, depuis la Konsole du compositeur, qui a déjà le bon `DISPLAY` : `xterm &`, `xeyes &`. Depuis
un autre terminal, il faut donner l'affichage du journal : `DISPLAY=:1 xterm`.

### Zen

Zen est installé en Flatpak (`app.zen_browser.zen`). Ses droits comprennent déjà les sockets
`wayland` et `fallback-x11`, et le dossier personnel. Flatpak lui passe la socket du compositeur,
lue dans `WAYLAND_DISPLAY`. Deux précautions :

- `--new-instance` : sinon, si Zen tourne déjà dans la session hôte, la commande lui confie la
  page, et la fenêtre s'ouvre dans la session hôte, hors du compositeur.
- `--profile` : un profil à part, parce que celui de tous les jours est verrouillé par le Zen de
  la session hôte. Le dossier d'essai ci-dessous est dans les données de Zen, que le bac à sable
  voit.

En Wayland natif, ce que Zen choisit seul quand `WAYLAND_DISPLAY` est défini :

```sh
mkdir -p ~/.var/app/app.zen_browser.zen/compositeur-essai
cargo run -p compositeur -- --imbrique flatpak run app.zen_browser.zen \
  --new-instance --profile "$HOME/.var/app/app.zen_browser.zen/compositeur-essai"
```

En X11, par XWayland : on retire la socket Wayland au bac à sable et on lui donne celle de X11.

```sh
cargo run -p compositeur -- --imbrique flatpak run --nosocket=wayland --socket=x11 \
  --env=MOZ_ENABLE_WAYLAND=0 app.zen_browser.zen \
  --new-instance --profile "$HOME/.var/app/app.zen_browser.zen/compositeur-essai"
```

Le journal dit lequel des deux tourne : `fenêtre X11 title="Zen Browser"` en X11, rien de tel en
Wayland. Ferme Zen avant le compositeur : fermé d'abord, le compositeur peut laisser des
processus de Zen derrière lui (`ps -eo pid,args | grep compositeur-essai` les montre).

### Presse-papiers en ligne de commande

`wl-copy` et `wl-paste` (paquet `wl-clipboard`) sont des clients Wayland, `xclip` un client X11.
Il faut qu'une fenêtre X11 ait le clavier, dans les deux sens : lancer le compositeur avec
`xterm`, et ne pas cliquer ailleurs. Avec les numéros du journal :

```sh
WAYLAND_DISPLAY=wayland-1 wl-copy "venu de Wayland"; DISPLAY=:1 xclip -o -selection clipboard
echo -n "venu de X11" | DISPLAY=:1 xclip -i -selection clipboard; WAYLAND_DISPLAY=wayland-1 wl-paste
```

Avec `--primary` pour `wl-copy` et `wl-paste`, et `-selection primary` pour `xclip`, c'est la
sélection primaire.

## Mesurer

`--mesurer`, juste après `--imbrique`, écrit chaque seconde dans le journal le nombre d'images,
la cadence, le temps de rendu moyen et maximal, le plus long intervalle entre deux images et le
nombre d'images en retard, c'est-à-dire arrivées plus de 16,67 ms après la précédente (cadence
de 60 images par seconde, `motion::FRAME_RATE`). Le temps de rendu va de la construction des
éléments à la fin du travail de la puce graphique : en mesure, le compositeur attend la fin du
rendu avant de présenter l'image. Quand la commande lancée se termine, la session s'arrête et
écrit deux bilans : toutes les images, et celles dessinées pendant une animation.

La scène de mesure, `examples/scene.rs`, ouvre dix fenêtres centrées l'une sur l'autre, puis
agrandit et rend sa taille à la plus haute toutes les 700 ms, douze fois : chaque changement est
un calage animé, avec ses dix ombres et ses dix découpes. Elle dure une dizaine de secondes.

```sh
cargo build --release -p compositeur --bin compositeur --example scene
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json \
  target/release/compositeur --imbrique --mesurer target/release/examples/scene
```

Sur le portable de référence, le compositeur doit tourner sur la puce AMD :

- le journal de Smithay nomme le moteur au démarrage, `GL Renderer: "AMD Radeon Graphics
  (radeonsi, …)"` ;
- `__EGL_VENDOR_LIBRARY_FILENAMES` ne laisse charger à libglvnd que l'EGL de Mesa : celui de
  NVIDIA, prioritaire sur Debian (`10_nvidia.json`), n'est même pas ouvert ;
- `cat /sys/bus/pci/devices/0000:01:00.0/power/runtime_status` doit rester `suspended` avant,
  pendant et après (adresse de la carte NVIDIA donnée par `lspci -D | grep -i nvidia`).

Sans `DRI_PRIME`, Mesa rend sur la puce par défaut, celle de l'écran : l'AMD. `DRI_PRIME=0` est
refusé par Mesa (`Invalid value (0) for DRI_PRIME`) : ne pas le mettre. Le journal du moteur
fait foi.

Sous Plasma X11, Mesa cale la présentation de la fenêtre hôte sur l'écran du portable : la
cadence plafonne à sa fréquence, et le temps de rendu compte aussi l'attente de l'image
précédente, si bien qu'il approche l'intervalle entre deux images. Pour voir le coût réel du
rendu, `vblank_mode=0` (variable de Mesa) lève cette attente. Au niveau `debug`
(`RUST_LOG=info,compositeur=debug`), chaque image en retard est journalisée avec son instant,
son intervalle et son temps de rendu : un retard dont le rendu est court vient d'ailleurs que du
rendu.

Le critère est « 60 images par seconde tenues » : aucune image en retard dans le bilan des
images animées. La cadence elle-même suit l'écran hôte : la fenêtre winit est présentée au
rythme de l'écran du portable. Les chiffres mesurés sont dans la demande de fusion du lot D-11.

## Comparer à la planche Main

1. Ouvrir `docs/design/planches/Main.dc.html` dans Chromium, fenêtre de 1440 par 900, ou en
   tirer une capture :
   `chromium --headless --disable-gpu --hide-scrollbars --window-size=1440,900 --screenshot=planche.png docs/design/planches/Main.dc.html`.
2. Lancer le compositeur sans commande, mettre sa fenêtre hôte à 1440 par 900, puis seulement
   ouvrir la fenêtre de la taille de l'éditeur de la planche (836 × 652). Le compositeur centre
   une fenêtre à son premier contenu, sur l'écran qu'il a à ce moment : l'ordre compte. Le nom
   de la socket est dans le journal (`compositeur prêt socket="wayland-1"`).

   ```sh
   cargo build -p compositeur --bin compositeur --example scene
   target/debug/compositeur --imbrique &
   wmctrl -r Smithay -e 0,-1,-1,1440,900
   WAYLAND_DISPLAY=wayland-1 target/debug/examples/scene planche &
   import -window "$(xwininfo -name Smithay | awk '/Window id/ {print $4}')" compositeur.png
   ```

   Vérifier que la capture fait 1440 par 900. Pour un hôte de L par H, la fenêtre est en
   ((L - 836) / 2, (H - 652) / 2) : son coin bas gauche est en ((L - 836) / 2, (H + 652) / 2).

3. Comparer :
   - le fond et les disques, au pixel près : mêmes places, mêmes couleurs ;
   - le coin bas gauche de la fenêtre : (36, 716) sur la planche, (302, 776) dans le compositeur
     à 1440 par 900. Les deux coins sont sur le disque `#132A49`. Recadrer autour de chacun et
     poser les deux côte à côte : rayon, bordure claire de 1 px (`(39, 47, 61)` sur le corps de
     la fenêtre dans les deux), ombre décalée vers le bas qui s'éteint en 70 px.

## Tester

```sh
just verifier
```

| Test | Ce qu'il vérifie |
| --- | --- |
| `src/layout.rs` | Centrage, déplacement, redimensionnement par chaque bord, coin opposé fixe pendant tout le geste, tailles minimale et maximale du client. Bande de saisie : chaque côté, chaque coin, plus large aux coins, rien à l'intérieur, fenêtre étroite. Calage : bords haut, gauche et droit, quatre coins, géométrie de chaque zone, retour à la taille d'avant sous le pointeur |
| `sherlock-ui/src/color.rs` | Couleurs prémultipliées par leur opacité |
| `sherlock-ui/src/motion.rs` | Ressorts : convergence vers la cible, dépassement léger et borné (entre 0,5 et 4 %), temps de réponse (90 % en moins de 150 ms, posé en moins de 300 ms), changement de cible sans saut de valeur ni de vitesse, pas de dépassement pour un ressort critique ou sur-amorti. Fondu : durée d'un survol, reprise sans saut |
| `src/anim.rs` | Ouverture centrée qui part d'une opacité nulle. Calage animé de la position puis de la taille, glisser suivi sans délai avant et après. Drapeau consommé même sans changement ou pendant une animation, arrêt par un geste. Aperçu qui apparaît, glisse et s'efface |
| `src/render.rs` | Disques à leur place de la planche sur 1440 par 900, ronds sur un autre écran. Dimensions de l'ombre de la fenêtre de la planche. Rayon des coins, réduit pour une petite fenêtre. Découpe retournée avec l'image |
| `src/stats.rs` | Cadence, images en retard, bilan des images animées |
| `tests/sans_ecran.rs` | Sans écran ni backend, voir ci-dessous |
| `tests/xwayland.rs` | XWayland et un client X11, sans écran ni backend, voir ci-dessous |

Les tests de `tests/sans_ecran.rs` :

- **Configuration.** Un client crée une fenêtre titrée et reçoit sa configuration. Il est agrandi,
  passe en plein écran, revient, puis se ferme.
- **Déplacement par la barre de titre.** Un client affiche un contenu : sa fenêtre est centrée.
  Il demande un déplacement avec un mauvais serial, qui est refusé. Avec le serial de son appui,
  la fenêtre suit le pointeur jusqu'au relâchement.
- **Calage en haut.** Lâchée contre le bord haut, la fenêtre est agrandie : l'aperçu couvre
  l'écran et le client reçoit 800 × 600 avec `maximized`. Déplacée de nouveau, elle reprend sa
  taille d'avant.
- **Calage dans un coin, puis sur un côté.** La fenêtre lâchée dans le coin bas droit prend ce
  quart. Lâchée ensuite contre le bord gauche, elle prend la moitié gauche. Super + bas la rend
  à sa place d'avant le glisser.
- **Agrandie avant son contenu.** Déplacée, elle reçoit 0 × 0 sans `maximized` : le client
  choisit. Calée puis rendue, elle reçoit encore 0 × 0, pas la taille de l'écran.
- **Bande de saisie.** Le curseur change au-dessus de la bande. Un appui dans le coin bas droit
  redimensionne la fenêtre. Un clic droit pendant le geste ne le termine pas.
- **Redimensionnement demandé par le client.** Un `xdg_toplevel.resize` par le coin bas droit
  fait suivre la taille.
- **Plein écran.** Ni déplacement ni redimensionnement, et pas de bande.
- **Raccourcis.** Super + flèches cale à gauche, agrandit, rend la taille d'avant. Le client,
  qui a un clavier, reçoit Super mais jamais l'appui ni le relâchement des flèches
  interceptées. Super + Maj + gauche lui arrive et ne cale rien.
- **Ce qui est dessiné suit les gestes.** `State::window_visual` donne la géométrie dessinée à
  un instant choisi, sans rendu. Agrandie puis rendue pendant l'animation, la fenêtre glissée
  ensuite est dessinée sous le pointeur à chaque image. Calée à gauche deux fois de suite, puis
  redimensionnée par sa bande, elle est dessinée à sa nouvelle taille. Calée puis détachée en
  la glissant vite, elle reste sous le pointeur.

Le test d'intégration fait tourner le compositeur et le client dans le même fil, sur une paire de
sockets Unix : il ne dépend ni d'un écran, ni de `XDG_RUNTIME_DIR`. Il charge `libwayland-client`
à l'exécution, comme le backend winit.

Le test de `tests/xwayland.rs` lance un vrai XWayland (il faut `Xwayland` sur le poste, et la CI
installe le paquet `xwayland`). La boucle du compositeur tourne dans le fil du test, un client X11
écrit avec x11rb dans un autre. Le client ouvre une fenêtre de 200 × 100 titrée et un menu
override-redirect en (50, 60). Le test vérifie, en regardant à la fois l'espace du compositeur
et ce que le serveur X sait de la fenêtre :

- le titre, et la fenêtre centrée, à la même place pour le compositeur et pour le serveur X ;
- le clavier donné à la fenêtre dès qu'elle a sa surface ;
- le menu à la place qu'il a choisie, dessiné tel quel, sans prendre le clavier ;
- Super + glisser jusqu'au bord haut : fenêtre agrandie, `_NET_WM_STATE` compris, sur tout
  l'écran ; Super + bas : retour à sa place d'avant ;
- la bande de saisie : le coin bas droit tiré, le coin haut gauche en place ;
- un déplacement demandé par la fenêtre (`_NET_WM_MOVERESIZE`) : refusé pendant un appui sur le
  menu, qui est pourtant du même client Wayland ; accordé pendant un appui sur la fenêtre ;
- le presse-papiers : la fenêtre sans le clavier ne pose ni CLIPBOARD ni PRIMARY côté Wayland ;
  avec le clavier, elle pose les deux ;
- la fermeture : le client part, ses fenêtres disparaissent, personne n'a le clavier ;
- l'arrêt propre : XWayland retire sa socket, et la socket Wayland du compositeur (ouverte dans
  le dossier temporaire, la CI n'ayant pas de `XDG_RUNTIME_DIR`) et son verrou disparaissent.

Ce que les tests ne voient pas : les programmes GLSL et l'image qu'ils produisent (la CI n'a pas
de puce graphique), la forme réelle du curseur de la fenêtre hôte, et le transfert des données du
presse-papiers entre Wayland et X11, vérifié à la main et en ligne de commande (voir « Lancer »). Pour ça, il faut la
procédure à la main ci-dessous, la mesure et la comparaison à la planche.

## Essai à la main

En mode imbriqué, sous Plasma X11, la touche des gestes est Alt :

```sh
RUST_LOG=compositeur=debug cargo run -p compositeur -- --imbrique env QT_QPA_PLATFORM=wayland konsole --separate
```

1. **Frappe et focus.** Tape dans Konsole. Ouvre une deuxième Konsole (Ctrl + Maj + N), puis
   clique d'une fenêtre à l'autre.
2. **Curseur de la bande.** Approche lentement le pointeur du bord droit de Konsole, par
   l'extérieur. À quelques pixels du bord, le curseur devient une double flèche horizontale.
   Fais le tour : double flèche verticale en haut et en bas, flèches diagonales près des coins,
   même un peu en diagonale hors de la fenêtre.
3. **Redimensionner par la bande.** Pour chacun des quatre côtés et des quatre coins, appuie juste
   à l'extérieur de la fenêtre et glisse. Le bord ou le coin suit le pointeur, et le coin
   opposé ne bouge pas. Le journal affiche `début du geste` avec `source=Border`.
4. **Bords de Qt.** Le redimensionnement par les 3 pixels intérieurs que Qt dessine marche
   toujours (`source=Client`).
5. **Curseur de Konsole.** Au-dessus du texte, le curseur est celui que Konsole demande,
   normalement une barre de texte. Au-dessus de la barre de titre, une flèche.
6. **Déplacer.** Glisse la barre de titre : le curseur prend la forme « déplacer » pendant le
   geste. Fais la même chose avec Alt + glisser.
7. **Caler en haut.** Glisse la barre de titre jusqu'au bord haut de la fenêtre du compositeur.
   Dépasser ce bord compte aussi : sous X11, la fenêtre hôte suit le pointeur tant que le bouton
   est appuyé. Un rectangle vert translucide couvre toute la zone. Lâche : Konsole est agrandie.
8. **Retour.** Reprends la barre de titre d'une Konsole agrandie, du côté droit, et glisse vers
   le bas. Elle reprend sa taille d'avant, et le pointeur reste sur sa barre de titre, du côté
   droit.
9. **Moitiés.** Glisse jusqu'au bord gauche, puis jusqu'au bord droit. L'aperçu couvre la moitié
   visée, et la fenêtre la prend au lâcher.
10. **Quarts.** Glisse jusque dans chacun des quatre coins. L'aperçu couvre ce quart, et la
    fenêtre le prend au lâcher. Près d'un coin, le long du bord, ça compte encore comme le coin.
11. **Alt + glisser.** Refais l'étape 7 avec Alt + glisser au milieu de la fenêtre.
12. **Fenêtres qui se chevauchent.** Place une Konsole en partie au-dessus de l'autre. Appuie
    juste à l'extérieur du bord de celle du dessous, à un endroit couvert par celle du dessus.
    C'est la fenêtre du dessus qui reçoit le clic, le curseur reste celui de Konsole, et rien
    n'est redimensionné.
13. **Redimensionner une fenêtre calée.** Cale une fenêtre à gauche, puis tire sa bande à droite.
    Elle n'est plus calée : la déplacer ne change plus sa taille.
14. **Raccourcis.** Dans Konsole, tape `echo un deux trois` sans valider, et place le curseur du
    shell sur `deux` avec les flèches. Puis :
    - Alt + flèche gauche, puis Alt + flèche droite : moitié gauche, puis moitié droite ;
    - Alt + flèche haut agrandit, une deuxième fois rend la taille d'avant ;
    - Alt + flèche bas rend la taille d'avant.

    Pendant tout ça, le curseur du shell ne bouge pas, aucun `;3D` ni `;3C` ne s'affiche, et rien
    ne se répète après le relâchement. Alt + Maj + flèche gauche, elle, ne cale rien : la touche
    va à Konsole.
15. **Simple clic.** Sur une Konsole agrandie, un Alt + clic sans bouger ne change rien.
16. **Plein écran.** Passe Konsole en plein écran (Ctrl + Maj + F11). Ni Alt + glisser, ni Alt +
    clic droit, ni la bande ne la déplacent ni ne la redimensionnent. Alt + flèches ne change
    rien non plus. Ressors du plein écran.

Alt + flèches entre en conflit avec les applications qui s'en servent, en mode imbriqué
seulement : ces touches n'arrivent plus à l'application. Par exemple, dans un navigateur,
Alt + flèche gauche revient à la page précédente, et les IDE JetBrains changent d'onglet avec
Alt + flèche gauche ou droite. En session réelle, ce sera Super + flèches.

### Applications X11

Depuis la Konsole du compositeur (commande ci-dessus), qui a déjà le bon `DISPLAY` :

```sh
xterm -xrm 'XTerm*selectToClipboard: true' &
xeyes &
```

`selectToClipboard` fait copier à xterm la sélection de la souris dans CLIPBOARD, et coller
CLIPBOARD avec Maj + Inser et le clic du milieu : sans lui, xterm ne connaît que PRIMARY.

1. **Affichage.** xterm et xeyes apparaissent au centre, avec l'animation d'ouverture, les coins
   arrondis, la bordure et l'ombre. Les yeux suivent le pointeur.
2. **Focus au clic.** Clique d'une fenêtre à l'autre et tape : le texte va à celle cliquée. Le
   curseur de xterm est un bloc plein quand il a le clavier, creux sinon.
3. **Gestes.** Refais avec xterm les étapes 3, 6, 7, 9, 10, 11, 13 et 14 de la liste ci-dessus :
   bande de saisie, Alt + glisser, calage en haut, moitiés, quarts, redimensionnement d'une
   fenêtre calée, Alt + flèches. Le contenu de xterm suit la taille de la fenêtre.
4. **Menus.** Ctrl + clic gauche dans xterm : son menu s'ouvre sous le pointeur, sans ombre ni
   coins. Choisis une entrée : elle s'applique, et le clavier reste à xterm.
5. **Presse-papiers, Wayland vers X11.** Dans Konsole, sélectionne un mot et Ctrl + Maj + C. Clique
   dans xterm et Maj + Inser : le mot s'affiche.
6. **Presse-papiers, X11 vers Wayland.** Dans xterm, sélectionne un mot à la souris. Clique dans
   Konsole et Ctrl + Maj + V : le mot s'affiche.
7. **Sélection primaire.** Lance un xterm sans `-xrm`. Sélectionne un mot dans Konsole, puis
   clic du milieu dans ce xterm : le mot s'affiche. Et l'inverse. Enfin, dans Konsole, tape
   `echo -n intrus | xclip -i -selection clipboard`, puis Ctrl + Maj + V : c'est l'ancien
   contenu qui se colle, pas `intrus`, car aucune fenêtre X11 n'a le clavier.
8. **Fermeture.** `exit` dans xterm : la fenêtre disparaît, et le clavier revient à la fenêtre du
   dessus.
9. **Zen.** Ferme le compositeur et relance-le avec l'une des deux commandes de Zen (voir
   « Lancer »). Zen s'ouvre au centre. Navigue, ouvre un menu (clic droit dans la page, menu
   principal), passe en plein écran (F11) et reviens, agrandis la fenêtre avec Alt + flèche haut.
   Depuis un autre terminal, `DISPLAY=:1 xterm -xrm 'XTerm*selectToClipboard: true'` (numéro du
   journal) ouvre un xterm à côté : copie l'adresse de la page dans Zen et colle-la dans xterm.
10. **Arrêt.** Ferme la fenêtre du compositeur. `ls /tmp/.X11-unix/` ne montre plus la socket
    `X<N>` du journal, `ls $XDG_RUNTIME_DIR` ne montre plus `wayland-N` ni `wayland-N.lock`, et
    `pgrep Xwayland` ne trouve plus rien.
