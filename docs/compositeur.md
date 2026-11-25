# Compositeur

Le compositeur affiche les fenêtres des applications Wayland et reçoit le clavier et la souris.
Il est écrit en Rust avec Smithay 0.7, dans `desktop/compositeur`. Le lot D-10 en pose la base :
il tourne en fenêtre dans une session existante, sans effet visuel.

## Architecture

| Fichier | Rôle |
| --- | --- |
| `src/main.rs` | Lit les options, crée la boucle d'événements, lance la commande donnée |
| `src/lib.rs` | `State` : affichage Wayland, protocoles, siège, `Space` des fenêtres. Socket, écran, focus |
| `src/handlers.rs` | Réponses aux protocoles : `wl_compositor`, `wl_shm`, `wl_seat`, `wl_output`, xdg-shell |
| `src/input.rs` | Clavier et souris : focus au clic, gestes Super + bouton |
| `src/layout.rs` | Disposition : placer, déplacer, redimensionner. Du calcul pur, sans Wayland ni rendu |
| `src/winit.rs` | Backend fenêtré : la fenêtre hôte, son écran virtuel, le rendu GLES |

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
- Super + glisser avec le bouton gauche déplace la fenêtre sous le pointeur.
- Super + glisser avec le bouton droit la redimensionne par le coin le plus proche du pointeur.
- Agrandie ou en plein écran, une fenêtre prend tout l'écran. Elle retrouve sa place en sortant.
- Les fenêtres dessinent leur barre de titre elles-mêmes : xdg-decoration est hors du lot.
- Le clavier est en AZERTY français, sauf si `XKB_DEFAULT_LAYOUT` est défini.

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

`QT_QPA_PLATFORM=wayland` force Konsole à parler Wayland : dans une session X11, Qt choisirait X11
et s'ouvrirait hors du compositeur. `--separate` évite que la commande rejoigne une Konsole déjà
ouverte.

Sous KDE Plasma, KWin réserve par défaut Meta + glisser et Meta + clic droit à ses propres
fenêtres, y compris celle du compositeur. Pour essayer les gestes en mode imbriqué, change le
temps de l'essai la touche de modification des actions de fenêtre dans les réglages de KWin
(Gestion des fenêtres, actions des fenêtres).

## Tester

```sh
just verifier
```

| Test | Ce qu'il vérifie |
| --- | --- |
| `src/layout.rs` | Centrage, déplacement, redimensionnement par chaque bord, tailles minimale et maximale du client |
| `tests/sans_ecran.rs` | Sans écran ni backend : un client se connecte, crée une fenêtre titrée, reçoit sa configuration, est agrandi puis passé en plein écran, revient, se ferme |

Le test d'intégration fait tourner le compositeur et le client dans le même fil, sur une paire de
sockets Unix : il ne dépend ni d'un écran, ni de `XDG_RUNTIME_DIR`. Il charge `libwayland-client`
à l'exécution, comme le backend winit.

À la main, en mode imbriqué : taper dans le terminal, cliquer d'une fenêtre à l'autre, Super +
glisser, Super + clic droit, agrandir et passer en plein écran depuis le client.
