# ADR 0011 : Sherlock : un démon, un serveur MCP par domaine, un courtier dans le compositeur

- Statut : accepté
- Date : 2026-11-24
- Lot : plan, tableau « Décisions figées »

## Contexte

Sherlock agit sur le système à la demande de l'utilisateur. Il ne doit jamais pouvoir s'accorder lui-même une action, ni lire ou cliquer la fenêtre où l'utilisateur donne son accord.

## Décision

Sherlock est un démon de session, `sherlockd`. Chaque domaine du système expose ses outils par un serveur MCP. Chaque appel d'outil passe par un courtier d'accords, et la fenêtre des accords est dessinée par le compositeur.

## Conséquences

- L'interface et Sherlock appellent la même API.
- Chaque outil déclare son niveau : lecture, action ou destructif. Un outil sans niveau est traité comme destructif.
- Un test automatique prouve qu'aucun appel ne contourne le courtier (lot A-11).
- La forme de l'accord renforcé pour le niveau destructif se décide par ADR au lot A-11.

## Ce qui ferait changer d'avis

- Une action apparaît au journal sans accord associé.
- Le protocole MCP ne permet plus de typer les outils et leur niveau.
