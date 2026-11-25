# Essai D-00 : la barre en Slint

Prototype jetable du lot D-00, jumeau de `essais/toolkit-iced` : même barre de 36 px ancrée en
haut de l'écran par layer-shell, même horloge, mêmes trois bureaux, même menu « Fenêtre » de
cinq entrées avec le sous-menu « Déplacer vers le bureau », même animation d'ouverture.

Slint ne sait pas créer de surface layer-shell. Le prototype branche donc Slint sur Wayland par
son API publique `slint::platform` :

- `ui/barre.slint` décrit la barre, le menu et le sous-menu, avec toutes les valeurs du design
  dans le global `Theme` ;
- `src/wayland.rs` crée la surface layer-shell et les `xdg_popup` avec smithay-client-toolkit,
  transmet le pointeur à Slint et copie le rendu logiciel de Slint dans des tampons `wl_shm` ;
- `src/main.rs` crée les composants et relie leurs rappels à la boucle Wayland.

Limites de ce pont, qu'un shell durable devrait combler : échelle fixée à 1 (pas de
`wp_fractional_scale`), pas de clavier, pas de curseur personnalisé, rendu logiciel seulement,
pas d'accessibilité branchée.

## Lancer

Dans une session Wayland dont le compositeur gère `zwlr_layer_shell_v1` (KDE Plasma, sway,
Hyprland, niri...) :

```sh
cd essais/toolkit-slint
cargo run --release
```

Sans session Wayland, un KWin imbriqué dans une fenêtre suffit :

```sh
kwin_wayland --x11-display :0 --socket wayland-essai --width 1440 --height 900 &
WAYLAND_DISPLAY=wayland-essai cargo run --release
```

Avec `ESSAI_AUTO=1`, la barre suit le même scénario que le prototype iced : 4 s de repos,
puis dix ouvertures du menu, puis elle quitte.

## Mesurer

Le script de mesure est partagé avec le prototype iced :

```sh
cargo build --release
python3 ../toolkit-iced/mesurer.py --runs 5 target/release/toolkit-slint
```

## Dépendances

| Crate | Version | Pourquoi |
| --- | --- | --- |
| `slint` | 1.19.1 | Le toolkit évalué, sans backend : seulement le moteur de rendu logiciel et les polices du système |
| `slint-build` | 1.19.1 | Compile `ui/barre.slint` en Rust au moment du build |
| `smithay-client-toolkit` | 0.21.1 | Surface layer-shell, popups, pointeur et tampons partagés, ce que Slint ne fait pas lui-même |
| `chrono` | 0.4.42 | Heure locale de l'horloge |
