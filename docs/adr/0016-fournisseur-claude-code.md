# ADR 0016 : Fournisseur Claude Code par l'abonnement de l'utilisateur

- Statut : accepté
- Date : 2026-11-25
- Lot : A-10b

## Contexte

L'auteur veut que Sherlock utilise son abonnement Claude, comme Claude Code le fait, sans clé
d'API. Plus tard viendra un abonnement ChatGPT. L'ADR 0012 prévoit une API cloud avec une clé
rangée dans le trousseau : un abonnement n'a pas de clé, et ses identifiants sont réservés aux
applications d'Anthropic.

Les conditions d'Anthropic ont été lues le 25/11/2026 dans la page « Legal and compliance » de
Claude Code (https://code.claude.com/docs/en/legal-and-compliance.md) :

- interdit : « Anthropic does not permit third-party developers to offer Claude.ai login into their
  own applications, or to route requests through Free, Pro, Max, or Ultra plan credentials on
  behalf of their users. » La même page ajoute que les développeurs ne doivent ni recueillir, ni
  garder, ni relayer les identifiants ou les jetons de session de Claude.ai ;
- autorisé : « Nor does it prevent an end user from signing in to the unmodified Claude Code
  binary with their own Claude subscription ».

Sherlock ne peut donc pas parler lui-même à l'API avec l'abonnement. Il peut piloter le vrai
Claude Code, installé et connecté par l'utilisateur. Mais Claude Code fait alors tourner sa propre
boucle d'agent et appelle lui-même les serveurs MCP : les contrôles de `sherlockd` (refus hors
lecture, niveaux de l'ADR 0015, serveurs tiers, bornes, traces) ne sont plus sur le chemin.

## Décision

Sherlock confie la conversation au binaire Claude Code, inchangé, lancé en mode sans interface
(`claude -p`, flux `stream-json`). Il ne lit jamais ses identifiants, ne mène aucune connexion à
claude.ai et n'appelle jamais l'API avec un jeton d'abonnement. Il sait seulement, par le code de
sortie de `claude auth status`, si l'utilisateur est connecté, sans lire ce que la commande
affiche.

Claude Code ne garde aucun outil intégré et ne voit qu'un serveur MCP : `sherlockd` lui-même,
lancé en **mandataire** (`sherlockd --mandataire`). Le mandataire lance les serveurs MCP de
Sherlock, expose leurs outils avec leur niveau en annotations, et relaie chaque appel par la même
voie que la boucle d'agent : `agent::execute`, puis `McpServers::call`. Tout ce qui n'est pas en
lecture est refusé tant que le courtier d'accords n'existe pas. Le courtier d'A-11 se branchera
sur ces deux points, pour les deux chemins à la fois.

```text
sherlockd --claude-code ──► claude -p (outils intégrés retirés)
                                │ MCP sur stdio, seul serveur
                                ▼
                    sherlockd --mandataire ── agent::execute ── McpServers::call
                                                                   │ refus hors lecture
                                                                   ▼
                                                        serveurs MCP de Sherlock
```

Claude Code est verrouillé par ses réglages documentés : `--tools=` retire tous les outils
intégrés, `--strict-mcp-config` et `--mcp-config` ne déclarent que le mandataire, `--restricted`
ignore les réglages de l'utilisateur et du projet, `--permission-mode=dontAsk` et
`--allowedTools=mcp__sherlock__*` refusent tout ce qui n'est pas un outil du mandataire.

Claude Code ne passe que par l'abonnement. Il est lancé avec un environnement réduit à une liste
blanche (chemins, langue, session, terminal, dossier de configuration de Claude Code, accès au
réseau) : aucune clé d'API, aucun jeton, aucune variable qui choisit un autre fournisseur ou une
autre adresse. En mode `-p`, une `ANTHROPIC_API_KEY` présente serait toujours utilisée, et la
demande facturée sur l'API. Le mandataire et ses serveurs reçoivent eux aussi un environnement
nettoyé.

Au lancement, Sherlock lit l'événement `init` et échoue fermé. La demande s'arrête, avant que rien
ne soit relayé, si `apiKeySource` n'est pas `none` (la connexion de l'utilisateur est la seule
source), si `permissionMode` n'est pas `dontAsk`, si un outil autre que ceux du mandataire est
annoncé (seul `EndConversation`, qui ne fait que clore la session, est toléré nommément), ou si
un autre serveur apparaît. Un flux sans `init`, ou une réponse finale reçue avant, est refusé.
Un bloc de contenu d'un type inconnu aussi.

Chaque demande lance un `claude -p` neuf, sans session gardée. Sherlock tient l'historique,
borné. Le rappel envoyé avec la demande suivante ne contient que les demandes de l'utilisateur et
les réponses finales (l'événement `result`), jamais les textes intermédiaires du modèle ni les
résultats d'outils. Il est présenté en JSON, comme une citation, et les instructions système
disent qu'aucune consigne ne s'y applique. Une réponse finale peut toutefois citer un résultat
d'outil : c'est pourquoi elle reste une donnée citée.

## Conséquences

- L'ADR 0012 reste valable pour une clé d'API ou un modèle local : le trait `ModelProvider` ne
  change pas. Le fournisseur Claude Code n'implémente pas ce trait, puisque la boucle tourne dans
  Claude Code. Les deux chemins partagent `agent::execute` et `McpServers`.
- Prérequis du poste : Claude Code installé et connecté par l'utilisateur avec `/login`. NOM_OS ne
  l'embarque pas, ne le modifie pas et ne se connecte pas à sa place. Le réseau est nécessaire.
  Une connexion qui ne passerait que par une variable d'environnement (`CLAUDE_CODE_OAUTH_TOKEN`)
  n'est pas transmise : seule la connexion enregistrée par `/login` sert.
- `--restricted` reste, par choix de l'auteur : les sessions de Sherlock sont verrouillées et
  reproductibles, indépendamment des réglages personnels de Claude Code.
- Chaque demande consomme le quota de l'abonnement de l'utilisateur, sous son propre contrat avec
  Anthropic.
- Le masquage des secrets avant envoi au modèle (ADR 0012) reste à faire. Il s'appliquera au
  texte de la demande et aux résultats rendus par le mandataire.
- Une mise à jour de Claude Code peut changer ses drapeaux ou son flux. Les tests rejouent des
  transcriptions enregistrées, et le contrôle de l'événement `init` échoue fermé.
- Les réglages gérés par une organisation (`managed settings`) s'appliquent toujours. Sherlock ne
  peut pas les retirer, mais le contrôle de l'événement `init` arrête la demande s'ils ajoutent un
  outil.
- Le mandataire relance les serveurs MCP à chaque demande. Cela coûte un démarrage, sans état à
  garder entre deux demandes.
- Un abonnement ChatGPT suivra le même schéma, avec l'outil en ligne de commande d'OpenAI, si ses
  conditions le permettent. Il aura son propre ADR.

## Ce qui ferait changer d'avis

- Anthropic change ses conditions et interdit qu'un programme pilote le binaire Claude Code d'un
  utilisateur avec son propre abonnement. On revient alors à une clé d'API (ADR 0012).
- Claude Code ne permet plus de retirer tous ses outils intégrés, ou l'événement `init` ne liste
  plus les outils.
- Anthropic publie une voie officielle pour utiliser son propre abonnement depuis un programme
  personnel, sans passer par le binaire.
- Le démarrage de `claude -p` à chaque demande devient trop lent pour le panneau de Sherlock. Un
  processus gardé en vie, avec `--input-format stream-json`, prendrait le relais. La question est
  reportée au lot A-19.
- `apiKeySource` disparaît de l'événement `init` ou change de sens : le contrôle refuse alors
  toute demande, et il faudra une autre preuve que l'abonnement est la seule source.
