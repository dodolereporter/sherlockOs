# Image du conteneur de build de la distribution.
#
# Tout build de la distribution tourne ici, pour que le résultat ne dépende pas du poste.
# Deux constructions de cette image installent les mêmes versions de paquets :
#   - l'image de base est épinglée par son empreinte ;
#   - apt lit un instantané figé de l'archive Debian (snapshot.debian.org), pas le miroir courant.
# Pour monter de version, change l'empreinte et SNAPSHOT ensemble, puis reconstruis.
# Mode d'emploi : docs/build.md.

# debian:trixie-slim, index multi-architecture relevé le 2026-11-24.
FROM debian:trixie-slim@sha256:8aa5a7bf5a1955bddbe7d74fcdd0b91ab30e74362a8b73af672b0481b35b44a0

# Instant de l'archive Debian lue par apt. C'est celui à partir duquel l'image de base a été
# construite (voir son /etc/apt/sources.list.d/debian.sources) : un instantané plus ancien
# rétrograderait des paquets de la base, et apt refuse.
ARG SNAPSHOT=20261123T000000Z

# Utilisateur non root du conteneur. Le script de lancement passe l'uid et le gid du poste,
# pour que les fichiers écrits dans le dépôt monté appartiennent à son utilisateur.
ARG BUILDER_UID=1000
ARG BUILDER_GID=1000

ENV LANG=C.UTF-8
ARG DEBIAN_FRONTEND=noninteractive

# Sources apt figées. HTTP suffit : l'intégrité vient des signatures de l'archive, vérifiées
# avec le trousseau Debian de l'image. Les fichiers Release d'un instantané sont périmés
# par construction, d'où Check-Valid-Until: no.
RUN rm -f /etc/apt/sources.list /etc/apt/sources.list.d/* \
 && printf '%s\n' \
      "Types: deb" \
      "URIs: http://snapshot.debian.org/archive/debian/${SNAPSHOT}/" \
      "Suites: trixie trixie-updates" \
      "Components: main" \
      "Signed-By: /usr/share/keyrings/debian-archive-keyring.pgp" \
      "Check-Valid-Until: no" \
      "" \
      "Types: deb" \
      "URIs: http://snapshot.debian.org/archive/debian-security/${SNAPSHOT}/" \
      "Suites: trixie-security" \
      "Components: main" \
      "Signed-By: /usr/share/keyrings/debian-archive-keyring.pgp" \
      "Check-Valid-Until: no" \
      > /etc/apt/sources.list.d/instantane.sources \
 && printf '%s\n' \
      'Acquire::Retries "5";' \
      'APT::Install-Recommends "false";' \
      'APT::Install-Suggests "false";' \
      > /etc/apt/apt.conf.d/90builder

# Paquets, par usage. Les versions viennent de l'instantané ci-dessus.
#   Toolchain GNU : les prérequis du poste de construction de Linux From Scratch.
#   Noyau Linux : configuration, compilation, BTF (dwarves), modules, initramfs.
#   Sources : téléchargement et vérification des signatures.
#   QEMU : qemu-system-x86_64, qemu-img, et le micrologiciel UEFI (la cible est UEFI, ADR 0001).
#   Images disque : table GPT (sfdisk, paquet fdisk) et partition EFI en FAT écrite sans droits
#   root ni montage (mtools). Ajoutés au lot Q-04 pour l'image témoin du banc de boot.
RUN apt-get update \
 && apt-get install -y \
      bash binutils bison bzip2 coreutils diffutils file findutils gawk g++ gcc grep gzip \
      m4 make patch perl python3 sed tar texinfo xz-utils \
      bc cpio dwarves flex kmod libelf-dev libncurses-dev libssl-dev lz4 openssl rsync zstd \
      ca-certificates curl git gnupg wget \
      ovmf qemu-system-x86 qemu-utils \
      fdisk mtools \
 && rm -rf /var/lib/apt/lists/*

# Linux From Scratch exige que /bin/sh soit bash. Debian y met dash par défaut.
RUN ln -sf bash /usr/bin/sh

# -o accepte un uid ou un gid déjà pris dans l'image, par exemple le groupe users (100).
RUN groupadd -o -g "${BUILDER_GID}" builder \
 && useradd -o -m -u "${BUILDER_UID}" -g "${BUILDER_GID}" -s /bin/bash builder \
 && mkdir -p /depot /cache/sources \
 && chown builder:builder /depot /cache/sources

ENV CACHE_SOURCES=/cache/sources
USER builder
WORKDIR /depot
CMD ["bash"]
