# sherlock-ui

`desktop/sherlock-ui` est la bibliothèque visuelle du desktop : le seul endroit où s'écrivent les
couleurs, les tailles, les polices et les textes d'interface. Le lot D-01 la remplit à partir de
la section Identité visuelle de `docs/contexte.md` et des planches, qui font foi pour
l'apparence. Le toolkit est iced 0.14 en rendu logiciel (ADR 0014).

## Deux étages

| Étage | Modules | Dépend de |
| --- | --- | --- |
| Jetons | `color`, `metrics`, `typography`, `icon`, `logo`, `window` | Rien |
| iced | `theme`, `component`, `focus`, `gallery` | iced 0.14, derrière la fonction `iced` |

La fonction `iced` est active par défaut, parce que tous les programmes du shell la veulent. Le
compositeur, qui ne lit que des jetons, la coupe avec `default-features = false` et ne compile
pas iced. iced est compilé avec le seul moteur tiny-skia : sans la fonction `wgpu`, aucune
variable d'environnement ne peut choisir le GPU ni réveiller la carte NVIDIA.

## Jetons

Tous les jetons sont nommés par leur rôle, en anglais comme le reste du code.

- **`color`** : fonds (`DESKTOP`, `DESKTOP_DISCS`, `SYSTEM_SCREEN`, `DEEPEST`, `WINDOW`,
  `TITLE_BAR`, `SIDEBAR`, `CARD`, `TERMINAL_BLOCK`), verres (`GLASS`, `MENU_GLASS`,
  `DOCK_GLASS`), traits et voiles (`BORDER`, `MENU_BORDER`, `CARD_BORDER`, `DOCK_BORDER`,
  `SECONDARY_BORDER`, `DIVIDER`, `FIELD`, `HOVER`, `PRESSED`, `DISABLED`, `TOGGLE_OFF`,
  `SHADOW`, `MENU_SHADOW`), textes (`TEXT_PRIMARY`, `TEXT_SECONDARY`, `TEXT_TERTIARY`,
  `TEXT_LINE_NUMBER`, `TEXT_ON_ACCENT`, `TEXT_ON_CREAM`), accents (`ACCENT` et ses nuances de
  survol et d'appui, les quatre bleus, `CREAM`, `AMBER`, `SALMON`), états (`SELECTED_ROW`,
  `ACTIVE_ENTRY`, `FOCUS_RING`). `Color` sait se poser sur un fond (`over`, mélange sRGB comme le
  navigateur et tiny-skia) et calculer un contraste WCAG (`contrast`).
- **`metrics`** : barre du haut, fenêtre, panneau, menu, bouton, interrupteur, champ, carte,
  anneau de focus, icônes, dock, espacements par rôle (`GAP_TIGHT`, `GAP_ICON`, `GAP_INLINE`,
  `GAP_SECTION`, `GAP_ROW`, `PAGE_PADDING`) et durées. Les gammes du design (bouton de 28 à 36,
  rayon de 8 à 10, trait d'icône de 1,6 à 1,7…) sont vérifiées à la compilation.
- **`typography`** : familles, tailles, graisses, et les trois fichiers de police à charger.
- **`window`** : bande de saisie et calage des fenêtres, posés par D-17 pour le compositeur.

## Polices

Inter (version variable), Space Grotesk (version variable) et JetBrains Mono Regular sont
embarquées dans `desktop/sherlock-ui/polices/`, chacune avec sa licence OFL 1.1. Elles viennent
des dépôts de leurs auteurs : Inter 4.1 (`rsms/inter`), Space Grotesk (`google/fonts`, fichier
`SpaceGrotesk[wght].ttf`), JetBrains Mono 2.304. Les versions variables évitent un fichier par
graisse ; cosmic-text, le moteur de texte d'iced, applique l'axe `wght` demandé. Le nom interne
d'Inter variable est « Inter Variable » : c'est celui de `typography::UI_FAMILY`. Un programme
les charge au démarrage : `typography::FONTS` passé à `application(..).font(..)`.

## Thème et composants

`theme::theme()` construit le thème iced sur les jetons. Chaque composant a sa fonction de
style par état dans `theme`, et son constructeur dans `component` :

| Composant | Constructeur | Planche |
| --- | --- | --- |
| Bouton principal, secondaire | `primary_button`, `secondary_button` | Réglages, Panneaux |
| Bouton d'icône seule | `icon_button(icône, libellé, message)` | Barre de titre de Réglages |
| Interrupteur 44 par 26 | `toggle(valeur, message)` | Réglages |
| Champ de texte | `text_field` | Recherche de Réglages |
| Entrée de menu, raccourci, sous-menu, coche | `menu_item(libellé, Trailing, message)`, `open` | Menus |
| Menu, séparateur | `menu`, `menu_separator` | Menus |
| Étiquette de section | `section_label` | Panneaux, Main |
| Carte, panneau de verre | `card`, `glass_panel` | Réglages, Panneaux |
| Textes | `body`, `description`, `title`, `icon` | Toutes |

Un composant sans message est désactivé. Les composants cliquables rendent un `Control`, qui
devient un `Element` et accepte `status(..)` et `focus_ring()` pour imposer un état (galerie).

### Clavier

iced 0.14 ne sait focaliser que ses champs de texte. `focus::Focusable` enveloppe les autres
composants : il se déclare aux opérations de focus d'iced, déclenche l'action à Entrée ou
Espace, et dessine un anneau bleu clair de 2 px à 2 px du bord. Un clic efface l'anneau, comme
`:focus-visible` dans un navigateur. Le programme fait circuler le focus :

```rust
fn subscription(&self) -> Subscription<Message> {
    keyboard::listen().filter_map(focus::tab).map(Message::Focus)
}
// dans update : Message::Focus(direction) => return focus::advance(direction),
```

### Icônes

Une icône est un tracé SVG sur la grille de 24 (`icon::Icon::path`). Le cadre commun (trait de
1,7, bouts et angles arrondis) vient des jetons ; `component::icon` la dessine à la taille et à
la couleur voulues. Pour en ajouter une : une variante de `Icon`, son tracé, une place dans
`Icon::ALL`. Le test vérifie que chaque tracé reste dans la grille. Une icône seule cliquable
passe par `icon_button`, dont le libellé est obligatoire.

### Ajouter un composant

1. Les valeurs manquantes vont d'abord en jetons, dans `color` ou `metrics`, nommées par rôle.
2. Sa fonction de style par état va dans `theme`, sans couleur ni taille littérale.
3. Son constructeur va dans `component`. S'il est cliquable, il rend un `Control` pour avoir le
   focus clavier et l'anneau.
4. Chaque texte qu'il pose sur un fond entre dans le test de contraste de `theme`.
5. Il prend une ligne dans le tableau des états de la galerie (`gallery.rs`).

## Logo

Le logo n'a ni texte ni nom. Il est redessiné à la main sur une grille de 512, d'après la
planche générée par IA que l'auteur a fournie, sans tracé automatique :

| Forme | Valeur |
| --- | --- |
| Carré de fond | 512, rayon 96 |
| Anneau | Centre (256, 256), rayon 140, épaisseur 36 |
| Queue | Arc de rayon 80 et d'épaisseur 28, bouts ronds, de (340, 368) sur l'anneau jusqu'au point |
| Point | Centre (308, 256), rayon 22 |
| Version sans carré | Même dessin, cadré sur `96 96 320 320` |

Les fichiers sont dans `desktop/sherlock-ui/logo/` : `logo.svg` (anneau vert, point crème, carré
bleu nuit), `logo-creme.svg` (crème sur bleu nuit), `logo-nuit.svg` (bleu nuit sur crème),
chacun aussi en `-sans-fond`, et leurs PNG en 16, 32, 64, 256 et 512 dans `logo/png/`. Le bleu
nuit est `DESKTOP`, le vert `ACCENT`, le crème `CREAM` ; un test le vérifie. Les SVG et les PNG
s'écrivent avec `python3 exporter.py` depuis ce dossier (librsvg, déjà sur le poste).

Écarts voulus avec la planche : la queue de la planche s'effile jusqu'à une pointe ; ici elle
garde une épaisseur constante, plus lisible petit. Le point a la même place dans les trois
versions.

À 16 px, l'anneau de la version avec carré fait un peu plus d'un pixel : on reconnaît l'anneau
et le point, la queue se devine à peine. La version sans carré, plus grande dans le même
cadre, se lit bien à 16 px : c'est elle qu'il faut prendre pour les très petites tailles.

La planche source n'est pas dans le dépôt : elle est générée par IA, irrégulière, pèse 760 Kio
et n'a plus d'usage une fois le symbole redessiné. Ses mesures sont dans le tableau ci-dessus.

## Galerie

```sh
cargo run -p sherlock-ui --example galerie
```

Elle montre chaque composant dans ses cinq états (normal, survol, appui, focus, désactivé), sur
le fond du bureau puis sur un panneau de verre, puis le menu Fenêtre de la planche Menus et son
sous-menu, deux cartes de la planche Réglages, des boutons vivants, et les six versions du logo à
96, 64, 32 et 16 px. Tab et Maj+Tab déplacent le focus, Entrée ou Espace active.

Procédure de comparaison aux planches, sur le poste de l'auteur (Plasma X11) :

1. Lancer la galerie. La fenêtre fait 1440 par 900, comme les planches.
2. Ouvrir `docs/design/planches/Menus.dc.html` dans un éditeur : le menu « Menu Fenêtre » se lit
   dans les styles en ligne. Dans la galerie, cliquer « Déplacer vers le bureau ». Comparer :
   menu de 256 de large, marge 6, entrées de 30 et rayon 7, raccourcis en `#A9B4C6` à 12 px,
   séparateurs, sous-menu de 190 aligné sur son entrée, coche verte sur « 1 · Code ».
3. Ouvrir `Reglages.dc.html` de même. Comparer les cartes (`#152238`, rayon 14, marges 16 et 18),
   les interrupteurs (44 par 26, vert, rond sombre de 20), le bouton secondaire de 34 de haut,
   et le champ de recherche (`rgba(255,255,255,0.07)`, rayon 9).
4. Une mesure au pixel se fait sur la capture du test (ci-dessous), à l'échelle 2.

## Tests

`just verifier` lance tout, sans écran :

- **Contraste** (`theme`) : chaque texte de chaque composant, dans chacun de ses états, posé sur
  chaque fond où il peut paraître (bureau et ses trois disques, verre et menu posés sur chacun,
  fenêtre, barre de titre, panneau latéral, carte, écran système), doit atteindre 4,5 pour 1. Le
  calcul lit les styles réels, pose les fonds translucides et compare.
- **Cohérence des jetons** : gammes du design vérifiées à la compilation, nuances du vert, logo
  limité aux couleurs du thème, icônes dans leur grille.
- **Galerie** (`tests/galerie.rs`) : capture du rendu de la galerie, sous-menu ouvert, dans
  `target/tmp/galerie-tiny-skia.png` ; un clic déclenche son action ; Tab parcourt les
  composants en sautant les désactivés, et Entrée ou Espace les active.

La capture n'est pas comparée à une image de référence : elle s'écrit à chaque test pour être
regardée. Une comparaison au pixel près ferait échouer la CI au moindre changement de rendu.

## Écarts connus avec les planches

- **Flou** : le verre n'est qu'une teinte translucide. Le flou est le travail du compositeur.
- **Interlettrage** : iced 0.14 n'en a pas. Les étiquettes de section sont en majuscules sans
  les 0,06 em de la planche.
- **Sélection de menu** : la planche Menus pose l'entrée ouverte à 0,30 d'opacité,
  `contexte.md` dit 0,26. À 0,30, un raccourci en gris tertiaire sur cette ligne tombe à 4,46, contre
  4,81 à 0,26 : le jeton garde 0,26. Un test le montre.
- **Champ de texte** : l'indication est en gris secondaire, pas tertiaire : le tertiaire tombe à
  4,25 sur le champ posé sur le disque le plus clair. La planche ne montre pas l'icône de
  recherche dans le champ : iced 0.14 n'accepte qu'une icône de police dans `text_input`.
- **États absents des planches** : survol, appui, focus et désactivé ne sont dessinés nulle part.
  Choix faits : survol du vert éclairci de 18 %, appui assombri de 14 % ; voile blanc de 7 et
  12 % sur les éléments sans fond ; anneau de focus bleu clair `#8FC1E8` ; désactivé sans fond,
  texte tertiaire, bordure `DIVIDER`. L'interrupteur éteint n'a pas de planche : piste blanche à
  18 %, rond gris clair.
- **Animations** : iced change d'état d'un coup. Les survols de 110 à 150 ms et les ressorts ne
  sont pas faits (voir ci-dessous).
- **Accessibilité** : iced 0.14 n'expose rien aux lecteurs d'écran. Le libellé d'une icône seule
  est obligatoire dans l'API et s'affiche en bulle au survol, en attendant AccessKit (ADR 0014).
- **Rendu du texte** : cosmic-text lisse en niveaux de gris, sans lissage sous-pixel ; la graisse
  600 sort un peu plus épaisse que dans un navigateur (annexe de l'ADR 0014).

## Ce que le lot ne fait pas

- Les ressorts et le fondu d'opacité réutilisables que l'ADR 0014 attend de `sherlock-ui` : le
  brief du lot ne les demande pas. À planifier.
- layer-shell : c'est le lot D-20. `iced_exwlshell` 0.21 n'a pas été essayé.
- Le menu qui s'ouvre en `xdg_popup` : la galerie pose le sous-menu à côté du menu, comme la
  planche. Le placement réel est celui de la barre (D-20).
