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

# Compilations du noyau témoin en parallèle : tous les processeurs, sauf si TEMOIN_JOBS est fixé
jobs_noyau := env("TEMOIN_JOBS", num_cpus())

# Construit l'image témoin du banc de boot dans le conteneur : ok (écrit BOOT-OK) ou muet
temoin variante="ok":
    distro/builder/builder.sh lancer env TEMOIN_JOBS={{ jobs_noyau }} distro/tests/temoin/construire.sh {{ quote(variante) }}

# Boote une image disque UEFI dans QEMU et attend BOOT-OK (sans argument : l'image témoin, construite avant)
test-boot image="distro/tests/sortie/temoin.img":
    if [[ {{ quote(image) }} == distro/tests/sortie/temoin.img ]]; then {{ quote(just_executable()) }} temoin ok; fi
    distro/builder/builder.sh lancer env BOOT_DELAI="${BOOT_DELAI:-90}" BOOT_SORTIE="${BOOT_SORTIE:-}" distro/tests/boot.sh {{ quote(image) }}

# Vérifie que le banc échoue proprement : initramfs sans BOOT-OK, image absente
tester-banc:
    {{ quote(just_executable()) }} temoin muet
    distro/builder/builder.sh lancer distro/tests/tester-banc.sh
