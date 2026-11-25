# ADR 0004 : Desktop en Rust, sur Smithay et iced

- Statut : accepté. Réserve sur iced levée le 2026-11-25 par l'ADR 0014
- Date : 2026-11-24
- Lot : plan, tableau « Décisions figées »

## Contexte

Le desktop est entièrement maison : compositeur, shell et applications. Il faut un langage, une bibliothèque de compositeur Wayland et un toolkit d'interface. COSMIC suit déjà la voie Rust, Smithay et iced.

## Décision

Le desktop s'écrit en Rust. Le compositeur repose sur Smithay. Le toolkit d'interface est iced, sous réserve : le lot D-00 compare iced et Slint et tranche par son propre ADR.

## Conséquences

- Un seul langage pour le compositeur, le shell, les applications et Sherlock.
- Le code d'interface durable attend la conclusion de D-00. Le lot D-01 (`sherlock-ui`) en dépend.
- Pas de toolkit maison en 0.1.

## Ce qui ferait changer d'avis

- L'ADR du lot D-00 recommande Slint ou un autre toolkit.
- Smithay ne permet pas une fonction exigée par le plan, comme la synchronisation explicite ou le multi-GPU.
