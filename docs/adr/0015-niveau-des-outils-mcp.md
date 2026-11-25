# ADR 0015 : Le niveau d'un outil MCP se lit dans ses annotations standard

- Statut : proposé
- Date : 2026-11-25
- Lot : A-10

## Contexte

Chaque outil déclare son niveau : lecture, action ou destructif (ADR 0011). Un outil sans niveau est
traité comme destructif. Il faut un format pour cette déclaration, lu par `sherlockd` dans la
réponse à `tools/list`, puis par le courtier d'accords (A-11).

Le protocole MCP prévoit déjà, dans `annotations`, deux indications booléennes : `readOnlyHint`
(valeur par défaut `false`) et `destructiveHint` (valeur par défaut `true`, sans objet quand
l'outil est en lecture). L'autre voie serait un champ propre au projet, par exemple
`_meta.niveau: "lecture" | "action" | "destructif"`.

## Décision

Le niveau se lit dans les annotations standard :

| Annotations | Niveau |
| --- | --- |
| `readOnlyHint: true` | Lecture |
| `readOnlyHint` absent ou `false`, `destructiveHint: false` | Action |
| Tout le reste, absence d'annotations comprise | Destructif |

Une valeur qui n'est pas un booléen compte comme absente.

## Conséquences

- Les serveurs du projet n'ont rien d'autre à apprendre que le protocole : un serveur écrit avec
  n'importe quelle bibliothèque MCP déclare son niveau de la même façon.
- Les valeurs par défaut du protocole rejoignent notre règle : un outil muet est destructif.
- Le protocole présente ces annotations comme des indications à ne pas croire venant d'un serveur
  inconnu. Ici, seuls les serveurs livrés avec l'OS sont lancés : le niveau est du code du projet,
  relu comme tel. Un serveur tiers, s'il en vient un jour, demandera une autre règle.
- La lecture tient dans une fonction, `Level::from_annotations` de `sherlockd` : changer de format
  ne touche qu'elle et ses tests.

## Ce qui ferait changer d'avis

- Le courtier a besoin d'un niveau plus fin que trois valeurs, par exemple par argument.
- Le protocole change le sens ou les valeurs par défaut de ces annotations.
- On décide de lancer des serveurs MCP tiers.
