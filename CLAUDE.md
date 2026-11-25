Avant toute chose, lis `docs/contexte.md` en entier. Puis lis ce fichier, puis le brief de ton lot.

# NOM_OS

Un OS Linux construit depuis les sources, avec un desktop en Rust et un agent IA nommé Sherlock.
Le plan, les jalons et les lots sont dans `docs/plan.md`. Les décisions d'architecture sont dans `docs/adr/`.

## Arborescence

| Dossier | Contenu |
| --- | --- |
| `distro/` | Conteneur de build, recettes de paquets, fabrique d'images, tests de boot |
| `desktop/compositeur/` | Compositeur Wayland, sur Smithay (binaire) |
| `desktop/sherlock-ui/` | Couleurs, rayons, typographie, composants (bibliothèque) |
| `desktop/<programme>/` | Un crate par programme du shell et par application, à venir |
| `sherlock/sherlockd/` | Démon de l'agent Sherlock (binaire) |
| `sherlock/<serveur>/` | Courtier d'accords et un crate par serveur MCP, à venir |
| `docs/plan.md` | Plan, jalons, lots |
| `docs/contexte.md` | Tout ce qu'un agent doit savoir sur le projet |
| `docs/adr/` | Fiches de décision, à partir de `0000-modele.md` |
| `docs/design/` | Planches et images de Sherlock. Référence visuelle, ne la modifie pas |
| `essais/` | Prototypes jetables, hors du workspace principal |

À la racine : `Cargo.toml` du workspace, `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`,
`.editorconfig`, `justfile`.

## Règles de stabilité

1. La branche principale est toujours verte. Personne n'y pousse directement.
2. Un lot, une branche, une demande de fusion. Environ 600 lignes modifiées au plus, hors code généré.
3. La CI est obligatoire : format, clippy sans avertissement, tests, build de l'image, boot dans QEMU.
4. Chaque lot livre ses tests et sa page de doc.
5. Pas de `unsafe` sans commentaire qui le justifie. Pas de `unwrap` hors des tests.
6. Toute nouvelle dépendance est justifiée dans la demande de fusion, avec une version épinglée.
7. Toute décision d'architecture passe par un ADR dans `docs/adr/`.
8. Un agent qui doute s'arrête et pose sa question dans la demande de fusion. Il n'invente pas.
9. Rien qui touche au noyau, au pilote ou aux accords de Sherlock ne fusionne sans ton test sur la machine.
10. Trois agents en parallèle au maximum, sur des dossiers différents.
11. Aucune signature d'IA. Pas de ligne `Co-Authored-By`, pas de mention « Generated with », pas de nom de
    modèle ou d'agent, ni dans les commits ni dans les demandes de fusion. L'auteur est le compte git du dépôt.

## Branches et commits

- Une branche par lot : `lot/<ID>`, par exemple `lot/D-10`. Partie de la branche principale à jour.
- Une seule demande de fusion par lot, avec le compte rendu demandé par le brief.
- Message de commit : `<ID> : <ce qui change>`, au présent, en français.
  Exemple : `D-10 : ajoute le déplacement des fenêtres`.

## Vérification

Avant d'ouvrir une demande de fusion, `just verifier` doit passer. La CI lance la même commande
sur chaque demande de fusion et sur la branche principale (voir `docs/ci.md`). Le dépôt n'a pas de
protection de branche : ne fusionne jamais une demande dont la CI n'est pas verte.

| Commande | Ce qu'elle fait |
| --- | --- |
| `just verifier` | `verifier-format`, puis `clippy`, puis `tester` |
| `just verifier-format` | `cargo fmt --all -- --check` |
| `just formater` | Applique le format |
| `just clippy` | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| `just tester` | `cargo test --workspace --all-features` |
| `just construire` | `cargo build --workspace` |

Conteneur de build : `just builder`, `just dans-builder <commande>`, `just tester-builder`
(voir `docs/build.md`). Banc de boot : `just test-boot [image]`, `just temoin`, `just tester-banc`
(voir `docs/tests-boot.md`). La CI lance aussi le banc : `verifier` et `test-boot` doivent être verts.

Prérequis du poste : `rustup`, `just`, un compilateur C qui sert d'éditeur de liens
(`gcc` et `libc6-dev` sur Debian) et les bibliothèques du compositeur (`libxkbcommon-dev` et
`libwayland-dev` sur Debian). La version de Rust est lue dans `rust-toolchain.toml` et
s'installe toute seule au premier `cargo`. Ne la change pas : elle est aussi recopiée dans
`rust-version` du `Cargo.toml` racine, les deux changent ensemble.

## Code Rust

- Les lints sont définis une fois dans `[workspace.lints]` du `Cargo.toml` racine. Chaque crate les
  reprend avec `[lints] workspace = true`. Ne les désactive jamais pour faire passer la CI.
- Refusés : `unwrap`, `expect`, `println!`, `eprintln!`, `dbg!` hors des tests ; tout bloc `unsafe`
  sans commentaire `// SAFETY:` ; toute fonction `unsafe` publique sans section `# Safety`.
- Erreurs typées dans les bibliothèques, erreurs avec contexte dans les binaires. Journalisation avec `tracing`.
- Identifiants en anglais, commentaires et documentation en français. Pas d'accents dans les noms
  de fichiers et de dossiers.
- Aucune couleur, taille ou texte d'interface en dur hors de `desktop/sherlock-ui`.
- Pas de nom pour l'OS : écris `NOM_OS` là où il en faut un.

## Décisions

Les décisions figées du plan ont chacune leur ADR, de `0001` à `0013`. Pour en proposer une nouvelle,
copie `docs/adr/0000-modele.md` sous le numéro suivant. Un ADR n'est appliqué qu'une fois validé par l'auteur.
