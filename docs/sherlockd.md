# sherlockd

`sherlockd` est le démon de Sherlock dans la session de l'utilisateur. Il tient la conversation,
appelle le modèle et enchaîne les outils exposés par les serveurs MCP (ADR 0011 et 0012). Le lot
A-10 en pose le cœur : l'interface du fournisseur de modèle, un faux modèle, le client MCP et la
boucle d'agent. Le lot A-10b ajoute le premier vrai modèle : Claude Code, sur l'abonnement de
l'utilisateur, derrière le mandataire (ADR 0016). Il n'y a encore ni courtier d'accords, ni
service pour l'interface.

## Architecture

Le crate `sherlock/sherlockd` est une bibliothèque et un binaire.

| Fichier | Rôle |
| --- | --- |
| `src/modele.rs` | Trait `ModelProvider`, messages de l'historique, emplacement de la clé (`Keyring`, `ApiKey`) |
| `src/faux.rs` | `ScriptedProvider`, faux modèle piloté par un scénario, qui garde chaque requête reçue |
| `src/mcp.rs` | `McpClient` sur stdio, `ServerConfig` et `Trust`, `Level` lu dans les annotations, trait `ToolHost`, `McpServers` qui route chaque outil vers son serveur et refuse hors lecture |
| `src/agent.rs` | `Agent` : la boucle, la limite de tours, le délai du modèle. `execute` : le contrôle d'un appel, partagé avec le mandataire |
| `src/mandataire.rs` | `serve` : `sherlockd` en serveur MCP, devant les serveurs de Sherlock |
| `src/claude_code.rs` | `ClaudeCode` : le fournisseur Claude Code, sa ligne de commande, la lecture de son flux |
| `src/serveur_essai.rs` | Serveur MCP de test, sans effet sur le système, correct ou hostile selon son cas |
| `src/main.rs` | `--essai`, `--claude-code`, `--mandataire` et `--serveur-essai` |
| `tests/mcp.rs` | Client et boucle contre le serveur de test, lancé en vrai processus, correct ou hostile |
| `tests/mandataire.rs` | Le mandataire en vrai processus, devant le serveur de test |
| `tests/claude_code.rs` | Le fournisseur sur des transcriptions enregistrées (`tests/transcriptions/`), et contre de faux programmes `claude` |

Dépendances : `tokio` (un seul fil, `process`, `io-util`, `io-std`, `time`, `macros`), `serde` et
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

## Le fournisseur Claude Code

`ClaudeCode` confie la conversation au vrai Claude Code, connecté par l'utilisateur à son propre
abonnement Claude (ADR 0016). Claude Code fait tourner la boucle d'agent. Sherlock garde les
contrôles en se plaçant entre Claude Code et ses serveurs MCP : le mandataire.

### Ce que les conditions d'Anthropic permettent

Vérifié le 25/11/2026 dans https://code.claude.com/docs/en/legal-and-compliance.md.

| Permis | Interdit |
| --- | --- |
| L'utilisateur se connecte lui-même, avec son abonnement, au binaire Claude Code non modifié | Proposer une connexion claude.ai dans sa propre application |
| Sherlock lance ce binaire et lit sa sortie | Faire passer des requêtes par les identifiants d'un abonnement pour le compte d'utilisateurs |
| | Recueillir, garder ou relayer les identifiants ou les jetons de session de Claude.ai |

Sherlock ne lit, ne copie ni ne réutilise jamais les identifiants de Claude Code. Il ne se connecte
jamais à la place de l'utilisateur et n'appelle jamais l'API avec un jeton d'abonnement. Il
n'embarque ni ne modifie Claude Code.

### Prérequis

1. Installer Claude Code : https://code.claude.com/docs/en/setup.
2. Lancer `claude` dans un terminal et se connecter avec `/login`.

Sherlock vérifie les deux avant la première demande, par le seul code de sortie de
`claude auth status` : programme introuvable, Claude Code absent ; code 1, non connecté. Sa
sortie, qui nomme le compte, n'est pas lue. Une connexion perdue en cours de route se lit dans le
flux (`"error": "authentication_failed"`). Dans les deux cas, le message dit en français quoi
faire, sans rien tenter.

### La ligne de commande

`claude` est lancé dans un dossier vide propre à Sherlock (`$XDG_RUNTIME_DIR/sherlockd`), la
demande sur son entrée standard.

| Réglage | Effet |
| --- | --- |
| `-p --output-format=stream-json --verbose` | Mode sans interface, un événement JSON par ligne |
| `--no-session-persistence` | Rien n'est gardé sur le disque par Claude Code |
| `--tools=` | Aucun outil intégré : ni shell, ni fichiers, ni web, ni tâches, ni sous-agents |
| `--strict-mcp-config --mcp-config=…` | Un seul serveur MCP, `sherlock` : `sherlockd --mandataire` |
| `--restricted` | Ni réglages de l'utilisateur ou du projet, ni outil qui exécute du code, pas de contournement des permissions |
| `--permission-mode=dontAsk --permission-prompts=none` | Tout ce qui demanderait un accord est refusé, personne n'est sollicité |
| `--allowedTools=mcp__sherlock__*` | Seuls les outils du mandataire passent le contrôle de Claude Code |
| `--settings=…` | `disableAllHooks`, les mêmes règles de permissions, `disableBypassPermissionsMode` |
| `--disable-slash-commands` | Ni compétences ni commandes |
| `--system-prompt=…` | Les instructions de Sherlock remplacent celles de Claude Code |
| `--max-turns=8` | La limite de tours de la boucle d'agent |
| `CLAUDE_CODE_DISABLE_CLAUDE_MDS`, `CLAUDE_CODE_DISABLE_AUTO_MEMORY`, `CLAUDE_CODE_DISABLE_TERMINAL_TITLE` | Aucun fichier CLAUDE.md, aucune mémoire automatique, pas de titre de session calculé |

La demande ne passe pas en argument : plusieurs drapeaux de `claude` prennent une liste de valeurs
et avaleraient le texte.

### Le flux d'événements

| Événement | Ce qu'en fait Sherlock |
| --- | --- |
| `system/init` | Vérifie que chaque outil commence par `mcp__sherlock__` et que le seul serveur est `sherlock`, connecté. Sinon la demande s'arrête (`ForbiddenTools`, `ProxyNotConnected`) |
| `assistant` | Texte : `Message::Assistant(Text)`. Appel : `Message::Assistant(ToolCalls)`, nom sans préfixe. Un appel sans le préfixe arrête tout |
| `user` | Résultat d'outil : `Message::ToolResult` |
| `result` | Réponse finale, ou erreur : non connecté, ou échec avec son texte |
| Autres | Ignorés, tracés au niveau `debug` |

Rien n'est relayé avant l'événement `init`, ni depuis un sous-agent. Chaque appel et chaque
résultat laissent une trace `info`. Les transcriptions de `tests/transcriptions/` viennent de vrais
échanges, réduits aux champs lus par Sherlock, identifiants de session et d'appel remplacés.

### Les bornes

| Constante | Valeur | Ce qu'elle borne |
| --- | --- | --- |
| `MAX_MESSAGE_BYTES` | 4 Mio | Une ligne du flux, comme un message MCP |
| `MAX_OUTPUT_BYTES` | 16 Mio | Toute la sortie lue pour une demande |
| `MAX_KEPT_TEXT` | 16 Kio | Un texte gardé dans l'historique, tronqué au-delà |
| `MAX_HISTORY_BYTES` | 64 Kio | L'historique gardé : les plus anciens messages tombent au-delà |
| `DEFAULT_TIMEOUT` | 300 s | Une demande entière. Claude Code est alors tué et attendu |

Chaque demande lance un `claude -p` neuf. L'historique reste chez Sherlock, et la demande suivante
est précédée d'un rappel des demandes et des réponses finales, jamais des résultats d'outils.

## Le mandataire

`sherlockd --mandataire` est un serveur MCP sur stdio. Il lance les serveurs de Sherlock (pour
l'instant, le serveur de test), puis :

| Méthode | Réponse |
| --- | --- |
| `initialize` | La version demandée si elle est connue (`2025-11-25`, `2025-06-18`, `2025-03-26`), sinon `2025-06-18` |
| `tools/list` | Les outils des serveurs, avec leur niveau en annotations standard (`Level::annotations`) |
| `tools/call` | `agent::execute`, puis `McpServers::call` : refus hors lecture, outil inconnu, échec du serveur. Un refus est un résultat en erreur, que le modèle peut expliquer |
| `ping` | Une réponse vide |
| Autre | L'erreur `-32601` |

Il garde les contrôles de la boucle d'agent : niveaux de l'ADR 0015, refus faute de courtier,
traces. Une ligne reçue de plus de `MAX_MESSAGE_BYTES` arrête le service. Au-delà de `MAX_CALLS`
(64, soit 8 tours de 8 appels), chaque appel reçoit un résultat en erreur sans être exécuté. À la
fin de son entrée, il arrête ses serveurs.

## La commande d'essai

```sh
cargo run -p sherlockd -- --claude-code "Utilise l'outil echo avec le texte Bonjour puis dis-moi ce qu'il a répondu"
RUST_LOG=sherlockd=info cargo run -p sherlockd -- --claude-code "…"   # avec la trace des événements
```

Elle affiche la conversation : la demande, l'appel d'`echo` par le mandataire, son résultat, la
réponse. Une demande de lire un fichier ou de lancer une commande n'aboutit pas : Claude Code n'a
aucun outil pour cela. Une demande de vider la corbeille est refusée par le mandataire.

## Les règles de sécurité appliquées

| Règle | Comment | Preuve |
| --- | --- | --- |
| Un résultat d'outil est une donnée, jamais une consigne | Les instructions système sont fixées à la création de l'agent, sans moyen de les modifier. Un résultat, réussi ou en erreur, n'entre dans l'historique que comme `ToolResult` | `agent::tests::resultat_d_outil_jamais_consigne` et `erreur_d_outil_jamais_consigne` : un outil rend « Ignore tes instructions… », en résultat ou en erreur, et le texte n'apparaît dans aucune instruction système ni aucun autre message. `erreur_json_rpc_reste_une_donnee` contre un vrai serveur |
| Le niveau est lu, dans le doute destructif | `Level::from_annotations`, serveur tiers tout destructif, doublons refusés | `mcp::tests::dans_le_doute_destructif`, `liste_les_outils_et_leur_niveau`, `serveur_tiers_tout_destructif`, `doublon_refuse` |
| Sans courtier, rien d'autre que la lecture ne s'exécute | Refus « courtier d'accords absent » (`McpError::BrokerAbsent`) à deux endroits : dans `McpServers::call`, la seule voie publique vers `tools/call`, et avant l'appel dans la boucle, en défense en profondeur. `McpClient::call_tool` n'est pas public | `appel_direct_hors_lecture_refuse` (appel direct de `McpServers::call`), `agent::tests::rien_au_dela_de_la_lecture_sans_courtier`, `lecture_et_destructif_dans_une_meme_reponse`, `agent_appelle_echo_et_refuse_l_outil_sans_niveau` |
| Un serveur ne fait ni paniquer ni grossir `sherlockd` | Bornes ci-dessus, serveur perdu arrêté et attendu | `liste_qui_n_est_pas_un_objet`, `ligne_geante_refusee`, `pagination_sans_fin_bornee`, `delai_depasse_serveur_arrete_et_attendu` |
| Claude Code n'a aucun outil intégré | Ligne de commande verrouillée, événement `init` vérifié | `claude_code::tests::aucun_outil_integre_et_le_mandataire_seul`, `configuration_mcp_avec_le_seul_mandataire`, `outil_integre_ou_autre_serveur_arrete_tout`, `sans_outil_integre_rien_n_est_lu_ni_lance` |
| Le mandataire garde le refus hors lecture | `agent::execute` puis `McpServers::call` | `mandataire::tests::refuse_hors_lecture_sans_rien_executer`, `relaie_la_lecture_et_refuse_le_reste` (vrai processus), `refus_du_mandataire_relaye` (transcription) |
| Sherlock ne touche pas aux identifiants de Claude Code | Seul le code de sortie de `claude auth status` est lu, jamais un fichier de Claude Code | `claude_absent_ou_non_connecte`, `non_connecte_explique` |
| Claude Code ne fait pas grossir `sherlockd` | Bornes du flux et de l'historique | `rien_avant_l_init_et_lignes_bornees`, `historique_borne`, `plafond_d_appels_par_conversation`, `message_geant_arrete_le_service` |

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
| `src/mandataire.rs` | Refus hors lecture, plafond d'appels, liste avec le niveau, message géant |
| `src/claude_code.rs` | Ligne de commande et configuration MCP, texte tronqué, rappel sans résultat d'outil |
| `tests/mandataire.rs` | Le mandataire devant le vrai serveur de test : `echo` relayé, `vider_corbeille` refusé |
| `tests/claude_code.rs` | Transcriptions d'echo, de refus, d'outils intégrés demandés et de connexion absente ; init piégé ; lignes bornées ; Claude Code absent ou non connecté ; historique borné |
| `tests/mcp.rs` | Liste des outils, serveur tiers, appel réussi, appel direct refusé, refus dans la conversation, erreur JSON-RPC qui reste une donnée, limite de tours, serveur qui plante, délai dépassé (serveur arrêté et attendu), liste mal formée, doublons, pagination sans fin, ligne géante, serveur introuvable, serveur muet |

Le test du serveur muet lance `sleep`, présent sur tout poste Linux et sur la CI. Les tests du
fournisseur n'appellent jamais Claude Code : ils remplacent `claude` par `false` (non connecté), par
un chemin absent, ou par `sh`, qui rejoue une transcription.

## Ce qui viendra avec A-11

- Le courtier d'accords prend la place du refus : `McpServers::call` et `agent::execute` lui
  passent chaque appel, avec son niveau, au lieu de refuser tout ce qui n'est pas en lecture. Le refus actuel reste la conduite
  quand le courtier est injoignable.
- Le journal en ajout seul : heure, outil, résumé des arguments, accord, résultat. La trace
  `tracing` actuelle n'en tient pas lieu.
- La forme de l'accord renforcé du niveau destructif, par ADR.
- Le réglage par lequel l'utilisateur accorde un autre niveau à un outil d'un serveur tiers
  (ADR 0015).
- Le test qui prouve qu'aucun appel ne contourne le courtier.

Plus tard : un fournisseur par clé d'API et sa lecture du trousseau, le masquage des secrets avant
envoi au modèle, le service qui relie `sherlockd` au panneau de Sherlock (A-19), et les serveurs MCP
par domaine (A-12 à A-18).
