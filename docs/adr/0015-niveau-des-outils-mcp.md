# ADR 0015 : Le niveau d'un outil MCP se lit dans ses annotations standard, pour nos serveurs seulement

- Statut : accepté
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

Une annotation est une déclaration du serveur lui-même. Pour un serveur livré avec l'OS, c'est du
code du projet, relu comme tel. Pour un serveur tiers, c'est la parole de la partie qu'on veut
contrôler : un serveur malveillant se dira en lecture.

## Décision

Pour nos propres serveurs MCP, le niveau se lit dans les annotations standard :

| Annotations | Niveau |
| --- | --- |
| `readOnlyHint: true`, `destructiveHint` absent ou `false` | Lecture |
| `readOnlyHint` absent ou `false`, `destructiveHint: false` | Action |
| `readOnlyHint: true` et `destructiveHint: true` (contradiction) | Destructif |
| Une valeur qui n'est pas un booléen, y compris `null` | Destructif |
| Tout le reste, absence d'annotations comprise | Destructif |

Un serveur tiers n'est jamais cru sur parole : ses annotations sont ignorées et tous ses outils
sont destructifs, sauf réglage explicite de l'utilisateur.

Un serveur est tiers par défaut. Seule une marque explicite dans sa configuration de lancement le
déclare comme l'un des nôtres (`ServerConfig::ours`, `Trust::Ours` dans `sherlockd`).

Un même nom d'outil listé deux fois, par un serveur ou par deux serveurs, est refusé : le serveur
n'est pas branché.

## Conséquences

- Nos serveurs n'ont rien d'autre à apprendre que le protocole : un serveur écrit avec n'importe
  quelle bibliothèque MCP déclare son niveau de la même façon.
- Les valeurs par défaut du protocole rejoignent notre règle : un outil muet est destructif. Toute
  ambiguïté va vers le niveau le plus restrictif.
- La lecture tient dans une fonction, `Level::from_annotations`, et la confiance dans
  `McpClient::list_tools`. Changer de format ne touche qu'elles et leurs tests.
- Le réglage par lequel l'utilisateur accorde un autre niveau à un outil tiers n'existe pas encore.
  Il viendra avec le courtier (A-11) ou plus tard.

## Ce qui ferait changer d'avis

- Le courtier a besoin d'un niveau plus fin que trois valeurs, par exemple par argument.
- Le protocole change le sens ou les valeurs par défaut de ces annotations.
- On décide de brancher des serveurs MCP tiers au-delà du tout destructif. Le niveau devra alors
  venir du projet, et non du serveur :
  - un manifeste installé avec l'OS associe l'identité du serveur (chemin et empreinte du binaire)
    et le nom de chaque outil à un niveau. Un outil absent du manifeste reste destructif ;
  - le serveur tourne dans un bac à sable ;
  - ses descriptions et ses schémas d'outils, qui partent au modèle, sont traités comme des
    données non fiables ;
  - un changement de sa liste d'outils demande une nouvelle validation.
