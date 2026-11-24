# ADR 0014 : Toolkit du shell

- Statut : proposé
- Date : 2026-11-24
- Lot : D-00

## Contexte

La barre, le dock, le lanceur et les notifications sont des programmes séparés, ancrés à l'écran
par layer-shell (ADR 0005). L'ADR 0004 retient iced sous réserve de cet essai. Le lot D-00 a
construit la même barre deux fois, en iced et en Slint, et les a mesurées. Les prototypes sont
dans `essais/toolkit-iced` et `essais/toolkit-slint`. Les mesures, leur méthode et la note
détaillée sont dans `0014-toolkit-du-shell-annexe.md`.

Ce qui contraint le choix : chaque programme du shell vit dans une surface layer-shell, avec des
menus en `xdg_popup` ; le portable de référence a deux cartes graphiques et la NVIDIA doit dormir
hors jeu ; l'animation vise 60 images par seconde ; tout se pilote au clavier ; la licence de
l'OS n'est pas encore choisie.

**Réserve sur les mesures.** Le poste tournait sous Plasma X11 : aucune session Wayland réelle
n'était disponible. Les chiffres viennent d'un KWin Wayland imbriqué dans une fenêtre X11. Ils
comparent les deux toolkits dans les mêmes conditions, mais ce ne sont pas ceux de la machine en
usage réel. Les cases « session réelle » de l'annexe restent à remplir.

## Décision

Le shell s'écrit en iced 0.14, avec la couche layer-shell de waycrate (`iced_layershell`,
renommée `iced_exwlshell`). Ses surfaces dessinent en rendu logiciel (tiny-skia) tant qu'une
mesure en session réelle ne montre pas qu'il faut le GPU, et jamais sur la carte NVIDIA.

Les mesures ne départagent pas vraiment les deux toolkits. En rendu logiciel, iced démarre en
118 ms contre 163 ms et occupe 11 Mio de mémoire résidente contre 17 Mio. Slint a un binaire
plus petit (8,7 Mo contre 12,1 Mo), ne redessine rien au repos quand iced redessine une fois par
seconde, et les deux suivent le rythme du compositeur pendant l'animation du menu. iced en wgpu
monte à 103 Mio.

Slint gagne sur la façon d'écrire l'interface :

- ses animations sont déclaratives (`animate opacity, y`), là où iced oblige à recalculer chaque
  couleur à la main, faute d'opacité de groupe ;
- son langage porte des propriétés d'accessibilité ; iced n'en a aucune ;
- il mélange les transparences comme le HTML des planches sans réglage.

iced gagne sur ce qui coûte le plus cher à long terme pour un shell :

- **layer-shell et popups existent déjà.** La couche de waycrate crée la surface, les popups
  imbriqués avec capture du pointeur, le clavier, la saisie de texte et l'échelle fractionnaire.
  Slint n'a rien de tout cela : le prototype a dû écrire sa propre plateforme Wayland (environ
  640 lignes), à laquelle manquent encore le clavier, la saisie de texte du lanceur, l'échelle
  fractionnaire, un rendu GPU et l'accessibilité, qui n'a pas d'API publique hors du backend winit.
  Les avantages de Slint sur l'accessibilité disparaissent donc dans une surface layer-shell.
- **Licence.** iced est sous MIT. Slint est sous GPLv3, sous licence commerciale, ou sous licence
  libre de redevances qui exige une mention de Slint et dont l'application à un OS reste à vérifier.
- **Précédent.** COSMIC fait tourner un bureau complet sur iced et layer-shell. Aucun bureau
  complet n'existe en Slint.

## Conséquences

- Le lot D-01 (`sherlock-ui`) part sur iced 0.14, avec la fonction `web-colors` pour que les
  transparences rendent comme les planches. Il doit fournir ce que iced n'a pas : des animations
  à ressorts et un fondu d'opacité réutilisables, pour qu'aucun composant ne refasse
  l'interpolation à la main.
- Chaque programme du shell fixe son moteur de rendu au démarrage. Sans réglage, iced prend la
  carte la plus puissante : sur la machine de référence, la barre réveille la NVIDIA et plante
  dans KWin imbriqué ; en préférant la puce économe, elle réveille quand même la NVIDIA et met 2
  à 4 s à s'afficher. En rendu logiciel, la première image arrive en 120 ms et la NVIDIA dort.
- iced redessine à chaque message : chaque minuteur du shell coûte une image. Les minuteurs
  doivent rester rares, et l'horloge ne devrait battre qu'à la minute.
- La couche de waycrate a changé de nom et cassé son API entre 0.19 et 0.21, en octobre 2026. Le
  prototype utilise 0.19.1 ; le lot qui écrit la vraie barre passe à `iced_exwlshell` 0.21 et
  épingle sa version. C'est un petit projet : il faut pouvoir le reprendre à notre compte s'il
  s'arrête.
- L'accessibilité n'est réglée par aucun des deux toolkits pour une surface layer-shell. Elle
  devient un risque suivi, à revoir avant la 0.1, avec le travail d'AccessKit dans le fork d'iced
  de COSMIC comme piste.
- Le flou derrière la barre ne dépend pas du toolkit : c'est le travail du compositeur.

## Ce qui ferait changer d'avis

- En session Plasma Wayland réelle, sur la machine de référence, la barre iced en rendu logiciel
  ne tient pas 60 images par seconde pendant l'animation du menu, ou dépasse nettement la
  mémoire de la barre Slint.
- La couche layer-shell de waycrate est abandonnée, ou casse encore son API sans qu'on puisse la
  reprendre.
- Slint publie un backend layer-shell avec popups, ou une API d'accessibilité pour les
  plateformes personnalisées : le coût du pont disparaît et Slint passe devant.
- La licence de l'OS est choisie et rend celle de Slint sans contrainte, alors que iced ne publie
  toujours qu'une version par an et que nos besoins attendent dans sa branche principale.
