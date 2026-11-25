# Essai D-00 : la barre en iced

Prototype jetable du lot D-00. Il reproduit la barre du haut de la planche
`docs/design/planches/Main.dc.html` : 36 px de haut, ancrée en haut de l'écran par layer-shell,
avec une horloge, trois bureaux dont un actif et un bouton « Fenêtre » qui ouvre un menu de
cinq entrées. L'entrée « Déplacer vers le bureau » ouvre un sous-menu. Le menu et le sous-menu
sont de vrais `xdg_popup` imbriqués, rattachés à la surface de la barre.

Le prototype jumeau en Slint est dans `essais/toolkit-slint`. La comparaison et la décision
sont dans `docs/adr/0014-toolkit-du-shell.md` et son annexe de mesures.

## Lancer

Dans une session Wayland dont le compositeur gère `zwlr_layer_shell_v1` (KDE Plasma, sway,
Hyprland, niri...) :

```sh
cd essais/toolkit-iced
cargo run --release
```

Sur un portable à deux cartes graphiques comme la machine de référence, iced choisit par
défaut la carte la plus puissante, donc la NVIDIA. Pour rester sur la puce AMD :

```sh
# Rendu logiciel, sans carte graphique
ICED_BACKEND=tiny-skia cargo run --release
# Ou rendu wgpu limité au pilote Vulkan AMD
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.json WGPU_POWER_PREF=low cargo run --release
```

Sans session Wayland, un KWin imbriqué dans une fenêtre suffit :

```sh
kwin_wayland --x11-display :0 --socket wayland-essai --width 1440 --height 900 &
WAYLAND_DISPLAY=wayland-essai ICED_BACKEND=tiny-skia cargo run --release
```

## Mode automatique

Avec `ESSAI_AUTO=1`, la barre reste au repos 4 s, puis ouvre le menu dix fois : le sous-menu
s'ouvre 750 ms après le menu, tout se ferme à 1,25 s, et le programme quitte à la fin.
Ce mode sert aux mesures.

## Mesurer

`mesurer.py` mesure un binaire de l'extérieur, à partir du journal `WAYLAND_DEBUG` : temps
jusqu'à la première image, CPU et mémoire au repos, cadence de l'animation d'ouverture du menu.
Le même script mesure le prototype Slint.

```sh
cargo build --release
python3 mesurer.py --runs 5 target/release/toolkit-iced
```

## Dépendances

| Crate | Version | Pourquoi |
| --- | --- | --- |
| `iced` | 0.14.0 | Le toolkit évalué. Fonctions : rendu wgpu et tiny-skia, Wayland, couleurs « web » pour mélanger les transparences comme la planche HTML |
| `iced_layershell` | 0.19.1 | Surface layer-shell et popups pour iced. Dernière version sous ce nom, construite sur iced 0.14 |
| `chrono` | 0.4.42 | Heure locale de l'horloge |

`Cargo.lock` épingle `winit-core` et `winit-common` en 0.31.0-beta.2 : `iced_exdevtools` 0.19.1
ne compile pas avec la beta.4.
