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
