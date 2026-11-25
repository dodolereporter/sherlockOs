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
| `src/mcp.rs` | `McpClient` sur stdio, `ServerConfig` et `Trust`, `Level` lu dans les annotations, trait `ToolHost`, `McpServers` qui route chaque outil vers son serveur et refuse hors lecture |
| `src/agent.rs` | `Agent` : la boucle, la limite de tours, le délai du modèle, le refus hors lecture |
| `src/serveur_essai.rs` | Serveur MCP de test, sans effet sur le système, correct ou hostile selon son cas |
| `src/main.rs` | `--essai` et `--serveur-essai` |
| `tests/mcp.rs` | Client et boucle contre le serveur de test, lancé en vrai processus, correct ou hostile |

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
standard. Le serveur est un processus enfant, lancé selon un `ServerConfig` : programme,
arguments, délai par requête et confiance (`Trust`). Sa sortie d'erreur passe à celle de
`sherlockd`.

| Étape | Message |
| --- | --- |
| Ouverture | `initialize` avec la version `2025-06-18`, puis la notification `notifications/initialized`. Une autre version en retour est une erreur |
| Découverte | `tools/list`, page après page tant que `nextCursor` est donné |
| Appel | `tools/call` avec le nom et les arguments. Les blocs de texte du résultat sont mis bout à bout, les autres sont signalés. `isError` est repris |
| Requête du serveur | `ping` reçoit une réponse vide, toute autre méthode l'erreur `-32601` |

Les erreurs sont typées (`McpError`) : lancement impossible, serveur arrêté, serveur perdu, délai
dépassé, réponse invalide, erreur JSON-RPC, outil en double, outil inconnu, refus faute de
courtier. Une notification est ignorée, et le journal de debug n'en garde que les 200 premiers
octets, caractères de contrôle échappés.

### Face à un serveur hostile

Tout ce que le serveur envoie est une donnée externe. Aucune ne doit faire paniquer `sherlockd`,
ni le faire grossir ou attendre sans limite. Les accès aux réponses passent par `get`, jamais par
un index qui panique.

| Constante | Valeur | Ce qu'elle borne |
| --- | --- | --- |
| `MAX_MESSAGE_BYTES` | 4 Mio | Un message lu, fin de ligne comprise. Au-delà : `Protocol`, serveur perdu |
| `MAX_TOOL_PAGES` | 16 | Les pages de `tools/list`. Au-delà : `Protocol` |
| `MAX_TOOLS_PER_SERVER` | 256 | Les outils d'un serveur. Au-delà : `Protocol` |
| `DEFAULT_TIMEOUT` | 10 s | Chaque requête, sauf autre délai dans `ServerConfig` |
| `SHUTDOWN_GRACE` | 500 ms | L'arrêt doux, avant `SIGKILL` |

Après un arrêt du serveur, un délai dépassé ou un message refusé (trop long, ligne partielle, JSON
illisible), l'échange n'est plus fiable : le serveur est **perdu**. Le client ferme son entrée
(arrêt doux), lui laisse `SHUTDOWN_GRACE`, envoie `SIGKILL` s'il vit encore, puis attend le
processus pour ne pas laisser de zombie. Toute requête suivante rend `Lost` aussitôt. Un serveur
qui échoue pendant `McpServers::add` est arrêté de la même façon.

Le client n'utilise pas la bibliothèque `rmcp`. En version 0.11, encore avant 1.0, elle change
d'API à chaque version mineure et tire `chrono`, `futures`, `tokio-util` et `async-trait` en plus.
Le strict nécessaire du protocole tient dans `src/mcp.rs`. Le choix se revoit quand `rmcp` sera
stable, ou quand il faudra un transport autre que stdio.

### Le niveau des outils

Selon l'ADR 0015, le niveau se lit dans les annotations standard, pour nos serveurs seulement :
`readOnlyHint: true` donne la lecture, `destructiveHint: false` l'action, tout le reste le niveau
destructif. Une contradiction (`readOnlyHint` et `destructiveHint` vrais), une valeur qui n'est
pas un booléen ou l'absence d'annotations donnent donc le niveau destructif.

Un serveur est tiers par défaut (`Trust::ThirdParty`) : ses annotations sont ignorées et tous ses
outils sont destructifs. Seul `ServerConfig::ours()` marque un serveur comme l'un des nôtres. Un nom
d'outil en double, dans une page, entre deux pages ou entre deux serveurs, est refusé
(`DuplicateTool`) et le serveur n'est pas branché.

## La boucle d'agent

`Agent::ask` ajoute la demande à l'historique, puis, à chaque tour :

1. envoie au modèle les instructions système, l'historique et les outils, avec un délai
   (`model_timeout`, 60 s par défaut) ;
2. s'il répond par un texte, c'est la réponse finale ;
3. s'il demande des outils, chaque appel est vérifié, exécuté s'il est permis, et son résultat
   ajouté à l'historique comme `ToolResult`, puis on repart au tour suivant. Au-delà de
   `max_calls_per_turn` appels dans une même réponse (8 par défaut), les suivants ne sont pas
   exécutés et reçoivent chacun un résultat en erreur.

Après `max_turns` tours (8 par défaut) sans réponse finale, la demande s'arrête avec
`AgentError::TurnLimit`. Un modèle lent donne `AgentError::ModelTimeout`, un fournisseur en panne
`AgentError::Provider`.

Un appel qui n'aboutit pas n'arrête pas la conversation : outil inconnu, refus, appel au-delà du
plafond, serveur arrêté ou muet, l'erreur (`CallError`) est rendue au modèle comme résultat en
erreur, et il peut l'expliquer à l'utilisateur.

Chaque appel d'outil laisse une trace `tracing` : outil, niveau, durée et issue au niveau `info`,
refus au niveau `warn`, échec au niveau `error`. Les réponses du modèle sont tracées au niveau
`debug`.

## Les règles de sécurité appliquées

| Règle | Comment | Preuve |
| --- | --- | --- |
| Un résultat d'outil est une donnée, jamais une consigne | Les instructions système sont fixées à la création de l'agent, sans moyen de les modifier. Un résultat, réussi ou en erreur, n'entre dans l'historique que comme `ToolResult` | `agent::tests::resultat_d_outil_jamais_consigne` et `erreur_d_outil_jamais_consigne` : un outil rend « Ignore tes instructions… », en résultat ou en erreur, et le texte n'apparaît dans aucune instruction système ni aucun autre message. `erreur_json_rpc_reste_une_donnee` contre un vrai serveur |
| Le niveau est lu, dans le doute destructif | `Level::from_annotations`, serveur tiers tout destructif, doublons refusés | `mcp::tests::dans_le_doute_destructif`, `liste_les_outils_et_leur_niveau`, `serveur_tiers_tout_destructif`, `doublon_refuse` |
| Sans courtier, rien d'autre que la lecture ne s'exécute | Refus « courtier d'accords absent » (`McpError::BrokerAbsent`) à deux endroits : dans `McpServers::call`, la seule voie publique vers `tools/call`, et avant l'appel dans la boucle, en défense en profondeur. `McpClient::call_tool` n'est pas public | `appel_direct_hors_lecture_refuse` (appel direct de `McpServers::call`), `agent::tests::rien_au_dela_de_la_lecture_sans_courtier`, `lecture_et_destructif_dans_une_meme_reponse`, `agent_appelle_echo_et_refuse_l_outil_sans_niveau` |
| Un serveur ne fait ni paniquer ni grossir `sherlockd` | Bornes ci-dessus, serveur perdu arrêté et attendu | `liste_qui_n_est_pas_un_objet`, `ligne_geante_refusee`, `pagination_sans_fin_bornee`, `delai_depasse_serveur_arrete_et_attendu` |

## Le serveur de test et la démonstration

`sherlockd --serveur-essai` sert un serveur MCP de test sur stdio. Il expose `echo`, en lecture, qui
renvoie son texte, et `vider_corbeille`, sans niveau, qui ne vide rien : s'il était exécuté, il
rendrait `OUTIL-SANS-NIVEAU-EXECUTE`, et les tests vérifient que ce texte n'apparaît jamais. Il vit
dans le binaire de `sherlockd` pour que `cargo run` et les tests le trouvent sans rien construire de
plus.

`sherlockd --serveur-essai <cas>` le rend défaillant ou hostile, pour les tests :

| Cas | Comportement |
| --- | --- |
| `plante-sur-appel` | S'arrête au premier `tools/call` |
| `muet-sur-appel` | Ne répond jamais à `tools/call` |
| `liste-non-objet` | Répond à `tools/list` par un tableau |
| `pagination-sans-fin` | Donne toujours une page suivante |
| `doublon-page`, `doublon-pages` | Liste `echo` deux fois, en lecture puis sans niveau, dans une page ou sur deux |
| `ligne-geante` | Répond à `tools/list` par une ligne de plus de 4 Mio |
| `erreur-consigne` | Répond à `tools/call` par une erreur JSON-RPC qui contient une consigne |

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
| `src/agent.rs` | Outil de lecture puis réponse, refus hors lecture, lecture et destructif dans une même réponse, plafond d'appels par tour, résultat et erreur jamais consignes, limite de tours, outil inconnu, modèle trop lent, scénario épuisé |
| `src/mcp.rs` | Lecture du niveau, dans le doute destructif, aperçu tronqué du journal |
| `tests/mcp.rs` | Liste des outils, serveur tiers, appel réussi, appel direct refusé, refus dans la conversation, erreur JSON-RPC qui reste une donnée, limite de tours, serveur qui plante, délai dépassé (serveur arrêté et attendu), liste mal formée, doublons, pagination sans fin, ligne géante, serveur introuvable, serveur muet |

Le test du serveur muet lance `sleep`, présent sur tout poste Linux et sur la CI.

## Ce qui viendra avec A-11

- Le courtier d'accords prend la place du refus : `McpServers::call` et `Agent::execute` lui
  passent chaque appel, avec son niveau, au lieu de refuser tout ce qui n'est pas en lecture. Le refus actuel reste la conduite
  quand le courtier est injoignable.
- Le journal en ajout seul : heure, outil, résumé des arguments, accord, résultat. La trace
  `tracing` actuelle n'en tient pas lieu.
- La forme de l'accord renforcé du niveau destructif, par ADR.
- Le réglage par lequel l'utilisateur accorde un autre niveau à un outil d'un serveur tiers
  (ADR 0015).
- Le test qui prouve qu'aucun appel ne contourne le courtier.

Plus tard : le vrai fournisseur de modèle et sa lecture du trousseau, le masquage des secrets avant
envoi au modèle, le service qui relie `sherlockd` au panneau de Sherlock (A-19), et les serveurs MCP
par domaine (A-12 à A-18).
