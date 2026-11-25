# Compositeur

Le compositeur affiche les fenêtres des applications Wayland et reçoit le clavier et la souris.
Il est écrit en Rust avec Smithay 0.7, dans `desktop/compositeur`. Le lot D-10 en pose la base :
il tourne en fenêtre dans une session existante, sans effet visuel. Le lot D-17 rend les fenêtres
faciles à prendre en main : bande de saisie, curseurs, calage contre les bords de l'écran.

## Architecture

| Fichier | Rôle |
| --- | --- |
| `src/main.rs` | Lit les options, crée la boucle d'événements, lance la commande donnée |
| `src/lib.rs` | `State` : affichage Wayland, protocoles, siège, `Space` des fenêtres. Socket, écran, focus |
| `src/handlers.rs` | Réponses aux protocoles : `wl_compositor`, `wl_shm`, `wl_seat`, `wl_output`, xdg-shell, formes de curseur. Pose d'une fenêtre selon son état (`rearrange`) |
| `src/input.rs` | Clavier et souris : focus au clic, gestes, calage, raccourcis, forme du curseur |
| `src/layout.rs` | Disposition : placer, déplacer, redimensionner, bande de saisie, zones de calage, retour à la taille d'avant. Du calcul pur, sans Wayland ni rendu |
| `src/winit.rs` | Backend fenêtré : la fenêtre hôte, son écran virtuel, son curseur, le rendu GLES et l'aperçu du calage |

Les tailles de la bande et du calage, et les couleurs de l'aperçu, viennent de
`sherlock-ui` (modules `color` et `window`).

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

## Lancer

```sh
cargo run -p compositeur -- --imbrique
```

Une fenêtre s'ouvre dans la session en cours, Wayland ou X11. Tout ce qui suit `--imbrique` est une
commande lancée avec la socket du compositeur dans `WAYLAND_DISPLAY`. Pour un terminal :

```sh
cargo run -p compositeur -- --imbrique env QT_QPA_PLATFORM=wayland konsole --separate
```

Les journaux s'affichent sur la sortie d'erreur, au niveau `info` par défaut. `RUST_LOG` le règle,
par exemple `RUST_LOG=compositeur=debug,smithay=warn`. Le nom de la socket apparaît au démarrage.
Au niveau `debug`, chaque geste écrit son début et sa fin, avec son type (bords tirés, ou aucun
pour un déplacement) et sa source (touche des gestes ou demande du client). Une demande du client
refusée l'est aussi.

`QT_QPA_PLATFORM=wayland` force Konsole à parler Wayland : dans une session X11, Qt choisirait X11
et s'ouvrirait hors du compositeur. `--separate` évite que la commande rejoigne une Konsole déjà
ouverte.

## Tester

```sh
just verifier
```

| Test | Ce qu'il vérifie |
| --- | --- |
| `src/layout.rs` | Centrage, déplacement, redimensionnement par chaque bord, coin opposé fixe pendant tout le geste, tailles minimale et maximale du client. Bande de saisie : chaque côté, chaque coin, plus large aux coins, rien à l'intérieur, fenêtre étroite. Calage : bords haut, gauche et droit, quatre coins, géométrie de chaque zone, retour à la taille d'avant sous le pointeur |
| `sherlock-ui/src/color.rs` | Couleurs prémultipliées par leur opacité |
| `tests/sans_ecran.rs` | Sans écran ni backend, voir ci-dessous |

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

Le test d'intégration fait tourner le compositeur et le client dans le même fil, sur une paire de
sockets Unix : il ne dépend ni d'un écran, ni de `XDG_RUNTIME_DIR`. Il charge `libwayland-client`
à l'exécution, comme le backend winit.

Ce que les tests ne voient pas : le rendu de l'aperçu et la forme réelle du curseur de la
fenêtre hôte. Pour ça, il faut la procédure à la main ci-dessous.

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
