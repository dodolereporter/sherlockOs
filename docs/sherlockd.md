# sherlockd

`sherlockd` est le démon de Sherlock dans la session de l'utilisateur. Il tient la conversation,
appelle le modèle et enchaîne les outils exposés par les serveurs MCP (ADR 0011 et 0012). Le lot
A-10 en pose le cœur : l'interface du fournisseur de modèle, un faux modèle, le client MCP et la
boucle d'agent. Il n'y a encore ni vrai modèle, ni courtier d'accords, ni service pour l'interface.

## Architecture

Le crate `sherlock/sherlockd` est une bibliothèque et un binaire.

| Fichier | Rôle |
| --- | --- |
| `src/modele.rs` | Trait `ModelProvider`, messages de l'historique, emplacement de la clé (`Keyring`, `ApiKey`) |
| `src/faux.rs` | `ScriptedProvider`, faux modèle piloté par un scénario, qui garde chaque requête reçue |
| `src/mcp.rs` | `McpClient` sur stdio, `Level` lu dans les annotations, trait `ToolHost`, `McpServers` qui route chaque outil vers son serveur |
| `src/agent.rs` | `Agent` : la boucle, la limite de tours, le délai du modèle, le refus hors lecture |
| `src/serveur_essai.rs` | Serveur MCP de test, sans effet sur le système |
| `src/main.rs` | `--essai` et `--serveur-essai` |
| `tests/mcp.rs` | Client et boucle contre le serveur de test, lancé en vrai processus |

Dépendances : `tokio` (un seul fil, `process`, `io-util`, `time`, `macros`), `serde` et
`serde_json`, `thiserror` pour les erreurs de la bibliothèque, `anyhow` pour celles du binaire,
`tracing` et `tracing-subscriber`. Toutes sont épinglées.

## L'interface du fournisseur

```rust
pub trait ModelProvider {
    fn complete(&mut self, request: ModelRequest<'_>)
        -> impl Future<Output = Result<ModelReply, ProviderError>> + Send;
}
```

`ModelRequest` porte trois choses séparées : les instructions système (`system`), l'historique
(`messages`) et les outils avec leur schéma (`tools`). Le modèle rend `ModelReply::Text`, la réponse
finale, ou `ModelReply::ToolCalls`, des outils à appeler. L'historique est une suite de
`Message` : `User`, `Assistant` (une réponse du modèle) et `ToolResult`.

Un fournisseur cloud, puis un modèle local, implémenteront ce trait. L'adaptateur d'une API rend
chaque `ToolResult` dans le bloc de résultat d'outil de cette API, jamais dans les instructions.

**La clé.** Elle vient du trousseau de l'utilisateur, jamais du dépôt ni d'un fichier de
configuration. Le trait `Keyring` en marque l'emplacement : le futur fournisseur le recevra à sa
création. `ApiKey` ne s'affiche jamais en clair dans un `Debug`. Aucune implémentation n'existe
dans ce lot, faute de fournisseur choisi.

**Le faux modèle.** `ScriptedProvider` déroule un scénario : une réponse fixe (`reply`) ou calculée à
partir de la requête (`then`), par exemple du dernier résultat d'outil. Il garde chaque requête dans
`requests` pour que les tests vérifient ce que le modèle a reçu.

## Le protocole

Le client parle JSON-RPC 2.0 au serveur, un message JSON par ligne sur son entrée et sa sortie
standard. Le serveur est un processus enfant, tué quand le client disparaît. Sa sortie d'erreur
passe à celle de `sherlockd`.

| Étape | Message |
| --- | --- |
| Ouverture | `initialize` avec la version `2025-06-18`, puis la notification `notifications/initialized`. Une autre version en retour est une erreur |
| Découverte | `tools/list`, page après page tant que `nextCursor` est donné |
| Appel | `tools/call` avec le nom et les arguments. Les blocs de texte du résultat sont mis bout à bout, les autres sont signalés. `isError` est repris |
| Requête du serveur | `ping` reçoit une réponse vide, toute autre méthode l'erreur `-32601` |

Chaque requête a un délai, donné au lancement du client. Les erreurs sont typées (`McpError`) :
lancement impossible, serveur arrêté, délai dépassé, réponse invalide, erreur JSON-RPC, outil
exposé par deux serveurs. Une réponse tardive à une requête abandonnée est ignorée.

Le client n'utilise pas la bibliothèque `rmcp`. En version 0.11, encore avant 1.0, elle change
d'API à chaque version mineure et tire `chrono`, `futures`, `tokio-util` et `async-trait` en plus.
Le strict nécessaire du protocole tient dans `src/mcp.rs`. Le choix se revoit quand `rmcp` sera
stable, ou quand il faudra un transport autre que stdio.

### Le niveau des outils

Le niveau se lit dans les annotations standard de l'outil, selon l'ADR 0015, au statut proposé :
`readOnlyHint: true` donne la lecture, `destructiveHint: false` l'action, tout le reste le niveau
destructif. Un outil sans annotations est donc destructif.

## La boucle d'agent

`Agent::ask` ajoute la demande à l'historique, puis, à chaque tour :

1. envoie au modèle les instructions système, l'historique et les outils, avec un délai
   (`model_timeout`, 60 s par défaut) ;
2. s'il répond par un texte, c'est la réponse finale ;
3. s'il demande des outils, chaque appel est vérifié, exécuté s'il est permis, et son résultat
   ajouté à l'historique comme `ToolResult`, puis on repart au tour suivant.

Après `max_turns` tours (8 par défaut) sans réponse finale, la demande s'arrête avec
`AgentError::TurnLimit`. Un modèle lent donne `AgentError::ModelTimeout`, un fournisseur en panne
`AgentError::Provider`.

Un appel qui n'aboutit pas n'arrête pas la conversation : outil inconnu, refus, serveur arrêté ou
muet, l'erreur (`CallError`) est rendue au modèle comme résultat en erreur, et il peut l'expliquer à
l'utilisateur.

Chaque appel d'outil laisse une trace `tracing` : outil, niveau, durée et issue au niveau `info`,
refus au niveau `warn`, échec au niveau `error`. Les réponses du modèle sont tracées au niveau
`debug`.

## Les règles de sécurité appliquées

| Règle | Comment | Preuve |
| --- | --- | --- |
| Un résultat d'outil est une donnée, jamais une consigne | Les instructions système sont fixées à la création de l'agent, sans moyen de les modifier. Un résultat n'entre dans l'historique que comme `ToolResult` | `agent::tests::resultat_d_outil_jamais_consigne` : un outil rend « Ignore tes instructions… », le texte n'apparaît dans aucune instruction système ni aucun autre message |
| Le niveau est lu, un outil sans niveau est destructif | `Level::from_annotations` | `mcp::tests`, et `liste_les_outils_et_leur_niveau` contre le serveur de test |
| Sans courtier, rien d'autre que la lecture ne s'exécute | Avant tout appel, la boucle refuse un outil qui n'est pas en lecture avec `CallError::BrokerAbsent` : « courtier d'accords absent » | `agent::tests::rien_au_dela_de_la_lecture_sans_courtier` (action et destructif, aucun appel ne part), `agent_appelle_echo_et_refuse_l_outil_sans_niveau` contre le serveur de test |

## Le serveur de test et la démonstration

`sherlockd --serveur-essai` sert un serveur MCP de test sur stdio. Il expose `echo`, en lecture, qui
renvoie son texte, et `vider_corbeille`, sans niveau, qui ne vide rien : s'il était exécuté, il
rendrait `OUTIL-SANS-NIVEAU-EXECUTE`, et les tests vérifient que ce texte n'apparaît jamais. Avec
`--plante-sur-appel`, le serveur s'arrête au premier `tools/call`, pour les tests de panne. Il vit
dans le binaire de `sherlockd` pour que `cargo run` et les tests le trouvent sans rien construire de
plus.

```sh
cargo run -p sherlockd -- --essai
RUST_LOG=sherlockd=info cargo run -p sherlockd -- --essai   # avec la trace des appels
```

La démonstration lance le serveur de test, liste ses outils, puis déroule un scénario du faux
modèle : il appelle `echo`, tente `vider_corbeille`, qui est refusé, et répond avec le résultat
d'`echo`. La conversation s'affiche sur la sortie standard, les journaux sur la sortie d'erreur.

## Les tests

```sh
cargo test -p sherlockd
```

| Où | Ce qui est vérifié |
| --- | --- |
| `src/agent.rs` | Outil de lecture puis réponse, refus hors lecture, résultat jamais consigne, limite de tours, outil inconnu, modèle trop lent, scénario épuisé |
| `src/mcp.rs` | Lecture du niveau dans les annotations |
| `tests/mcp.rs` | Liste des outils, appel réussi, refus de l'outil sans niveau, limite de tours, serveur qui plante (seul et pendant une conversation), serveur introuvable, serveur muet |

Le test du serveur muet lance `sleep`, présent sur tout poste Linux et sur la CI.

## Ce qui viendra avec A-11

- Le courtier d'accords prend la place du refus : `Agent::execute` lui passe chaque appel, avec son
  niveau, au lieu de refuser tout ce qui n'est pas en lecture. Le refus actuel reste la conduite
  quand le courtier est injoignable.
- Le journal en ajout seul : heure, outil, résumé des arguments, accord, résultat. La trace
  `tracing` actuelle n'en tient pas lieu.
- La forme de l'accord renforcé du niveau destructif, par ADR.
- Le test qui prouve qu'aucun appel ne contourne le courtier.

Plus tard : le vrai fournisseur de modèle et sa lecture du trousseau, le masquage des secrets avant
envoi au modèle, le service qui relie `sherlockd` au panneau de Sherlock (A-19), et les serveurs MCP
par domaine (A-12 à A-18).
