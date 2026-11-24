# CI Rust

La CI refuse toute demande de fusion qui ne passe pas `just verifier`. Elle tourne sur GitHub Actions,
décrite dans `.github/workflows/ci-rust.yml`.

## Quand elle tourne

| Événement | Ce qui se passe |
| --- | --- |
| Demande de fusion, ouverte ou mise à jour | Une exécution. Un nouveau commit annule celle en cours |
| Commit sur `main` | Une exécution, menée au bout, qui enregistre aussi le cache |

## Ce qu'elle vérifie

Une seule tâche, `verifier`, lance `just verifier`. Elle enchaîne trois recettes du `justfile` et
s'arrête à la première qui échoue.

| Recette | Commande | Échoue si |
| --- | --- | --- |
| `just verifier-format` | `cargo fmt --all -- --check` | Un fichier n'est pas au format de `rustfmt.toml` |
| `just clippy` | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Clippy ou rustc émet un avertissement, ou un lint de `[workspace.lints]` est enfreint |
| `just tester` | `cargo test --workspace --all-features` | Un test échoue ou ne compile pas |

La CI n'a pas sa propre liste de commandes : elle lance les recettes du `justfile`. Pour changer ce
qu'elle vérifie, on change `just verifier`, et le poste local suit.

Le build de l'image et le boot dans QEMU, demandés par la règle 3 de `CLAUDE.md`, viennent avec les
lots Q-03 et Q-04.

## Comment elle s'installe

- **Rust.** `rustup toolchain install`, sans argument, installe la version et les composants écrits
  dans `rust-toolchain.toml`. Aucune version n'est écrite dans le workflow : changer
  `rust-toolchain.toml` suffit.
- **just.** Installé par `taiki-e/install-action`, version 1.59.0, épinglée dans le workflow.
- **Cache Cargo.** `Swatinem/rust-cache` garde le registre Cargo et le dossier `target`. Sa clé
  contient la version de rustc et le `Cargo.lock` : un changement de l'un ou de l'autre repart d'un
  cache neuf. Seules les exécutions sur `main` enregistrent le cache. Les demandes de fusion le
  relisent sans l'écraser.
- **Actions tierces.** Chacune est épinglée par l'empreinte complète de son commit, avec la version
  en commentaire. Pour en monter une, on remplace l'empreinte et le commentaire ensemble.
- **Machine.** `ubuntu-24.04`, nommée explicitement plutôt que `ubuntu-latest`, pour qu'un changement
  d'image ne casse pas la CI par surprise.
- **Durée maximale.** 15 minutes. Au-delà, l'exécution est arrêtée et compte comme un échec.

## La lancer en local

Avant d'ouvrir une demande de fusion :

```sh
just verifier
```

C'est exactement ce que lance la CI, avec la même version de Rust, lue dans `rust-toolchain.toml`.
Prérequis du poste : `rustup`, `just` et un compilateur C (`gcc` et `libc6-dev` sur Debian).

Pour lancer une étape seule :

```sh
just verifier-format   # format seul
just formater          # corrige le format
just clippy            # clippy seul
just tester            # tests seuls
```

## Lire un échec

Dans la demande de fusion, la vérification `CI Rust / verifier` passe au rouge. Le lien « Details »
ouvre le journal : la dernière recette lancée par `just` est celle qui a échoué, et sa sortie donne le
fichier et la ligne en cause. On corrige, on relance `just verifier` en local, on pousse.

On ne désactive jamais un lint ni un test pour faire passer la CI.

## Rendre la vérification bloquante

Le workflow marque la demande de fusion en échec, mais il ne retire pas le bouton de fusion. Pour que
GitHub refuse la fusion, la vérification `verifier` doit être déclarée obligatoire sur `main`, par une
règle de protection de branche ou un ensemble de règles (« Require status checks to pass »).

Au 24 novembre 2026, ce réglage n'est pas disponible sur ce dépôt : il est privé, et GitHub réserve la
protection des branches des dépôts privés à l'offre Pro. L'API répond « Upgrade to GitHub Pro or make
this repository public to enable this feature ». En attendant, la règle 1 de `CLAUDE.md` tient par
discipline : on ne fusionne pas une demande dont `CI Rust / verifier` n'est pas verte.
