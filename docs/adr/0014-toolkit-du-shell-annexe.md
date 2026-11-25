# Annexe à l'ADR 0014 : mesures et note de comparaison

Cette annexe donne les chiffres et les constats sur lesquels s'appuie l'ADR 0014. Les deux
prototypes sont dans `essais/toolkit-iced` et `essais/toolkit-slint`. Ils font la même chose :
une barre de 36 px ancrée en haut de l'écran par layer-shell, une horloge, trois bureaux dont
un actif, un bouton « Fenêtre » qui ouvre un menu de cinq entrées en `xdg_popup`, dont
« Déplacer vers le bureau » ouvre un sous-menu en `xdg_popup` imbriqué. Le menu s'ouvre par un
fondu et un glissé de 6 px en 180 ms, avec la même courbe.

## Conditions de mesure

Le poste de l'auteur tournait sous **Plasma X11** (`kwin_x11`, session de type x11, pas de
`WAYLAND_DISPLAY`). Aucune session Wayland réelle n'était disponible pour ce lot.

Les mesures ont donc été prises dans un **KWin Wayland 6.3.6 imbriqué** dans une fenêtre X11 de
1440 par 900, sur le portable de référence (Ryzen 7 avec puce AMD Renoir, RTX 3060, Mesa 25.0.7,
écran interne à 144 Hz). Ce KWin imbriqué propose `zwlr_layer_shell_v1` version 5 et les deux
prototypes s'y affichent normalement. Mais il rythme les images à environ 60 à 70 par seconde,
de façon irrégulière (écarts de 14 et de 20 ms qui alternent), et il compose à travers X11. Les
chiffres comparent les deux toolkits dans les mêmes conditions ; ils ne valent pas mesure en
session réelle.

Commande du compositeur imbriqué :

```sh
kwin_wayland --x11-display :0 --socket wayland-d00 --width 1440 --height 900 &
```

## Méthode

Tout est mesuré de l'extérieur par `essais/toolkit-iced/mesurer.py`, le même script pour les deux
prototypes. Il lance le binaire avec `WAYLAND_DEBUG=client` et lit le journal des requêtes
Wayland, qui a le même format pour les deux. Aucun chiffre ne vient du prototype lui-même.

| Mesure | Comment |
| --- | --- |
| Démarrage | Temps entre le lancement du processus et le premier `wl_surface.attach` sur la surface layer-shell, à l'horloge du script |
| Mémoire au repos | RSS et PSS lues dans `/proc/<pid>/smaps_rollup`, 7 s après la première image |
| CPU et images au repos | Temps CPU (`/proc/<pid>/stat`) et nombre d'images envoyées pendant 5 s, 2 s après la première image. La résolution du CPU est de 0,2 % |
| Taille du binaire | `cargo build --release`, profil par défaut, puis la même chose passée à `strip` |
| Images par seconde | Mode `ESSAI_AUTO=1` : dix ouvertures du menu par essai. Pour chaque ouverture, images envoyées sur la surface du menu pendant les 180 ms qui suivent sa première image, horodatées par le journal Wayland. Cadence = (images − 1) / durée. On garde aussi le plus long écart entre deux images |

Cinq essais par configuration, médiane retenue. Avant chaque essai, le script attend que la
carte NVIDIA soit endormie (`runtime_status` à `suspended`), pour que tous partent du même
état. Le journal `WAYLAND_DEBUG` coûte un peu de temps à chaque requête, pour les deux toolkits.

Trois configurations :

- **Slint** : moteur de rendu logiciel de Slint, tampons `wl_shm`.
- **iced tiny-skia** : `ICED_BACKEND=tiny-skia`, rendu logiciel.
- **iced wgpu AMD** : `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.json WGPU_POWER_PREF=low`,
  rendu Vulkan sur la puce AMD seule.

Pour reproduire, depuis la racine du dépôt :

```sh
(cd essais/toolkit-iced && cargo build --release)
(cd essais/toolkit-slint && cargo build --release)
python3 essais/toolkit-iced/mesurer.py --runs 5 essais/toolkit-slint/target/release/toolkit-slint
ICED_BACKEND=tiny-skia python3 essais/toolkit-iced/mesurer.py --runs 5 essais/toolkit-iced/target/release/toolkit-iced
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/radeon_icd.json WGPU_POWER_PREF=low \
  python3 essais/toolkit-iced/mesurer.py --runs 5 essais/toolkit-iced/target/release/toolkit-iced
```

## Tableau des mesures

### KWin imbriqué (mesuré)

| Mesure | Slint | iced tiny-skia | iced wgpu AMD |
| --- | --- | --- | --- |
| Démarrage jusqu'à la première image, médiane | 163 ms | 118 ms | 286 ms |
| Démarrage, cinq essais | 159, 177, 163, 170, 163 ms | 268, 117, 118, 122, 118 ms | 324, 286, 286, 289, 281 ms |
| Mémoire résidente au repos (RSS) | 16,6 Mio | 11,4 Mio | 103,1 Mio |
| Mémoire proportionnelle au repos (PSS) | 10,7 Mio | 8,9 Mio | 90,7 Mio |
| CPU au repos | 0,0 % | 0,0 % | 0,0 à 0,2 % |
| Images envoyées au repos | 0 | 1 par seconde | 1 par seconde |
| Taille du binaire | 10,8 Mo | 17,1 Mo | 17,1 Mo |
| Taille du binaire après `strip` | 8,7 Mo | 12,1 Mo | 12,1 Mo |
| Images par seconde pendant l'ouverture du menu, médiane | 71,5 | 62,9 | 63,3 |
| Images par seconde, pire ouverture | 53,3 | 56,8 | 51,4 |
| Plus long écart entre deux images, toutes ouvertures | 33,9 ms | 22,2 ms | 21,7 ms |
| Ouvertures mesurées | 50 sur 50 | 49 sur 50 | 50 sur 50 |

Le binaire iced est le même pour les deux colonnes : le moteur de rendu se choisit au lancement.
Les bibliothèques système (Mesa, libwayland, Vulkan) sont chargées à l'exécution et ne comptent
pas dans la taille. Le binaire Slint lie en plus fontconfig et freetype.

Les cadences au-dessus de 60 viennent du rythme irrégulier du KWin imbriqué (écarts de 14 et de
20 ms), pas des toolkits. Les deux suivent ce rythme ; aucun ne perd d'image de façon visible sur
une animation aussi simple. Le plus long écart de Slint (33,9 ms) correspond à une image sautée ;
sa pire ouverture reste à 53 images par seconde. Pour iced tiny-skia, une ouverture sur 50 n'a
pas donné deux images dans la fenêtre de 180 ms ; la cause n'a pas été cherchée. La cadence réelle à 144 Hz reste à mesurer.

Une première série Slint, sans le rendu à blanc des menus au démarrage (voir Animations), donnait
98 ms au démarrage et 13,6 Mio de RSS, mais la première ouverture de chaque essai apparaissait
sans animation. Le tableau donne la version corrigée, qui anime toutes les ouvertures.

### Démarrage selon le choix de la carte graphique (iced wgpu)

Essais séparés, avec la même mesure de la première image que `mesurer.py` (script réduit à cette
seule mesure, non livré), NVIDIA endormie au départ.

| Réglage | Première image | Carte NVIDIA après coup |
| --- | --- | --- |
| Aucun réglage (iced demande la carte la plus puissante) | Jamais : `wgpu error: Invalid surface`, le programme plante | Réveillée |
| `WGPU_POWER_PREF=low` | 2,0 s et 3,8 s | Réveillée : wgpu interroge tous les pilotes Vulkan |
| `WGPU_POWER_PREF=low` et pilote Vulkan AMD seul | 0,28 à 0,98 s | Endormie |
| `ICED_BACKEND=tiny-skia` | 0,12 s, 1,2 s au premier lancement | Endormie |

Slint en rendu logiciel ne touche à aucune carte graphique.

### Session Plasma Wayland réelle (non mesuré)

Aucune case n'est remplie : le poste n'avait pas de session Wayland pendant le lot. Pour les
remplir, ouvrir une session Plasma Wayland sur la machine de référence, puis lancer depuis la
racine du dépôt les trois commandes de la section Méthode, sans KWin imbriqué.

| Mesure | Slint | iced tiny-skia | iced wgpu AMD |
| --- | --- | --- | --- |
| Démarrage jusqu'à la première image | non mesuré | non mesuré | non mesuré |
| Mémoire au repos (RSS / PSS) | non mesuré | non mesuré | non mesuré |
| Images par seconde pendant l'ouverture du menu | non mesuré | non mesuré | non mesuré |
| Taille du binaire | identique au tableau ci-dessus | identique | identique |

## Note de comparaison

### Prise en charge de layer-shell

- **iced** n'a pas layer-shell dans son dépôt. La couche de waycrate (`iced_layershell` 0.19.1,
  renommée `iced_exwlshell` en 0.21) remplace la boucle winit d'iced par la sienne : surface
  layer-shell, ancrage, marges, zone exclusive, popups, clavier, saisie de texte, échelle
  fractionnaire, plusieurs écrans. Le prototype n'a eu qu'à remplir des réglages. Projet de
  petite taille (158 étoiles, licence MIT), très actif (138 commits en 90 jours, quatre versions
  en octobre et novembre 2026), mais qui a changé de nom et cassé son API entre 0.19 et 0.21.
  COSMIC suit une autre voie : son propre fork d'iced, avec layer-shell intégré.
- **Slint** n'a pas layer-shell. Son backend winit ne crée que des fenêtres ordinaires. Le
  prototype a dû écrire sa propre plateforme : smithay-client-toolkit pour Wayland, le moteur
  logiciel de Slint pour le rendu, environ 640 lignes de Rust dans `src/wayland.rs`. Il y manque encore le clavier, la
  saisie de texte, l'échelle fractionnaire, les curseurs, le rendu GPU et l'accessibilité. Deux
  crates tierces font ce pont, `layer-shika` (AGPL-3.0) et `spell-framework` (GPL-3.0, construite
  sur les crates internes de Slint 1.17) : leurs licences et leur dépendance aux API internes les
  écartent.
- Sur la machine de référence, iced en wgpu choisit d'office la carte la plus puissante et
  plante dans KWin imbriqué. Le réglage par variable d'environnement suffit, mais le shell devra
  fixer lui-même son moteur de rendu au démarrage.

### Menus et sous-menus

- Sur une barre de 36 px, un menu doit sortir de la surface : il faut un `xdg_popup` rattaché à
  la surface layer-shell, et un second `xdg_popup` imbriqué pour le sous-menu. Les deux
  prototypes le font.
- **iced** : `iced_layershell` crée le popup, le place, l'imbrique dans un autre popup et prend la
  capture du pointeur avec le numéro du dernier clic, pour que le menu se ferme quand on clique
  ailleurs. iced n'a pas de widget de menu : chaque menu est une vue écrite à la main.
- **Slint** : le langage a des menus (`MenuBar`, `ContextMenuArea`, `PopupWindow`), mais ils
  s'affichent dans la surface qui les ouvre, donc coupés à 36 px. Le prototype fait du menu et du
  sous-menu des composants à part, et sa plateforme crée les popups elle-même, capture comprise.
- Ni l'un ni l'autre ne gère le clavier dans les menus dans ces prototypes (flèches, Échap) :
  avec iced, la couche layer-shell le permet ; avec Slint, c'est encore du code de plateforme.

### Rendu du texte

- Les deux affichent le français sans faute (accents, « · », « ✓ » trouvé par repli dans une
  autre police), avec un lissage en niveaux de gris et sans lissage sous-pixel. Inter n'étant
  pas installée, les deux prennent Noto Sans.
- **iced** passe par cosmic-text (mise en forme harfrust, rastérisation swash) : repli de
  police, texte bidirectionnel. Le gras (600) sort un peu plus épais qu'en Slint.
- **Slint** en rendu logiciel passe par parley et fontique (mise en forme harfrust, contours
  skrifa) : même niveau de mise en forme. Rendu net, graisses un peu plus fines.
- iced mélange les couleurs transparentes en espace linéaire par défaut : un blanc à 14 % sur la
  barre sortait nettement plus clair que la planche. Il faut sa fonction `web-colors` pour
  retrouver le mélange du HTML de la planche. Slint mélange comme le navigateur d'office.

### Animations

- **Slint** : déclaratif. `animate y, opacity { duration: 180ms; easing: cubic-bezier(...) }`
  suffit, l'opacité s'applique à tout un sous-arbre. Deux pièges : une propriété lue pour la
  première fois ne s'anime pas (le prototype dessine les menus une fois à blanc au démarrage),
  et le moteur logiciel ne sait ni tourner ni mettre à l'échelle.
- **iced** : impératif. `Animation<bool>` donne la progression, l'abonnement `window::frames()`
  fournit les images, et chaque couleur doit être recalculée à la main : iced n'a pas d'opacité
  de groupe. C'est plus de code par composant animé.
- Aucun des deux n'a de vrais ressorts physiques : il faudra les écrire dans `sherlock-ui`.
- Au repos, Slint ne redessine que si une propriété change : 0 image par seconde. iced redessine
  après chaque message, donc à chaque tic de l'horloge : 1 image par seconde, pour un coût CPU
  sous la résolution de la mesure.

### Accessibilité

- **iced 0.14** n'a aucune accessibilité : pas d'AccessKit, pas d'arbre exposé aux lecteurs
  d'écran. Le fork d'iced de COSMIC y travaille avec AccessKit. La couche waycrate annonce une
  ouverture pour l'accessibilité dans sa version 0.21, sans l'implémenter.
- **Slint** a des propriétés `accessible-role`, `accessible-label`, `accessible-checked`... dans
  son langage, et les expose par AccessKit avec son backend winit. Le prototype les déclare. Mais
  la plateforme publique `slint::platform` n'offre aucun moyen de brancher l'accessibilité : avec
  layer-shell, ces propriétés ne vont nulle part.
- Pour un shell en layer-shell, aucun des deux ne règle donc la règle « une icône seule a
  toujours un libellé pour les lecteurs d'écran ». Slint a l'avance du modèle, pas du résultat.

### Activité du projet

Relevé le 24 novembre 2026 sur GitHub.

| | iced | Slint |
| --- | --- | --- |
| Étoiles | 32 121 | 24 715 |
| Commits sur 90 jours | 402 | 2 267 |
| Dernière version | 0.14.0, le 7 décembre 2025 | 1.19.1, le 24 novembre 2026 |
| Rythme des versions | Une par an environ | Environ tous les deux mois |
| Porteur | Un mainteneur principal et des contributeurs, adopté par System76 pour COSMIC | L'entreprise SixtyFPS GmbH |
| Licence | MIT | GPL-3.0, ou licence libre de redevances avec mention obligatoire de Slint, ou commerciale |

iced avance dans sa branche principale mais publie peu : la version épinglée a presque un an.
Slint publie souvent et garde une API stable depuis 1.0, mais sa licence demande une vérification
avant de l'embarquer dans un OS dont la licence n'est pas choisie.

### Flou

Le plan demandait un panneau avec flou. Ni iced ni Slint ne floutent ce qui est derrière leur
surface : c'est le travail du compositeur (protocole de flou de KWin, ou le nôtre au lot du
compositeur). Les deux prototypes dessinent un verre translucide sans flou. Le choix du toolkit
ne change rien à ce point.

## Choix faits pendant l'essai

- Le menu de cinq entrées est le menu « Fenêtre » de la planche Menus : Réduire, Agrandir,
  Moitié gauche, Déplacer vers le bureau (sous-menu des trois bureaux), Fermer la fenêtre.
- Les largeurs de la partie gauche de la barre sont fixes, comme sur la planche Menus, pour
  placer le menu sans relire la mise en page. Les icônes de la partie droite (conteneurs, réseau,
  son) ne sont pas reproduites : le brief ne les demande pas.
- Le bouton Sherlock est une pastille crème sans la mascotte.
- iced est mesuré sur la couche waycrate 0.19.1, dernière version publiée sous le nom
  `iced_layershell`. Le renommage en `iced_exwlshell` 0.21 a été découvert pendant l'essai ; la
  version 0.21 annonce moins d'appels à la vue, ce qui pourrait réduire les redessins au repos.
- Slint est mesuré en rendu logiciel : c'est le seul moteur que l'API publique laisse brancher
  sans écrire de code EGL. iced est mesuré dans ses deux moteurs.
- Les interactions à la souris (survol qui ouvre le sous-menu, clic hors du menu qui le ferme)
  sont écrites mais n'ont pas été essayées à la main : seul le mode automatique a tourné.
