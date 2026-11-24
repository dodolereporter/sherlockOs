set shell := ["bash", "-euo", "pipefail", "-c"]

# Liste les recettes
default:
    @just --list

# Format, clippy et tests : ce que la CI exige
verifier: verifier-format clippy tester

# Vérifie le format sans rien modifier
verifier-format:
    cargo fmt --all -- --check

# Applique le format
formater:
    cargo fmt --all

# Clippy sur tout le workspace, avertissements refusés
clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Lance tous les tests du workspace
tester:
    cargo test --workspace --all-features

# Compile tout le workspace
construire:
    cargo build --workspace

# Construit l'image du conteneur de build de la distribution
builder:
    distro/builder/builder.sh construire

# Lance une commande dans le conteneur de build, dépôt monté dans /depot (bash sans commande)
[positional-arguments]
dans-builder *commande:
    @distro/builder/builder.sh lancer "$@"

# Teste le conteneur de build : outils, utilisateur, cache des sources, codes de sortie
tester-builder:
    JUST='{{ just_executable() }}' distro/builder/tester.sh rapide

# Construit deux fois l'image sans cache et compare les versions des paquets (lent)
tester-builder-repro:
    distro/builder/tester.sh repro
