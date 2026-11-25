# Image of the build container of the distribution.
#
# Every build of the distribution runs here, so that the result does not depend on the host.
# Two builds of this image install the same package versions:
#   - the base image is pinned by its digest;
#   - apt reads a frozen snapshot of the Debian archive (snapshot.debian.org), not the live mirror.
# To move to newer versions, change the digest and SNAPSHOT together, then rebuild.
# How to use it: docs/build.md.

# debian:trixie-slim, multi-architecture index read on 2026-11-24.
FROM debian:trixie-slim@sha256:8aa5a7bf5a1955bddbe7d74fcdd0b91ab30e74362a8b73af672b0481b35b44a0

# Point in time of the Debian archive read by apt. It is the one the base image was built from
# (see its /etc/apt/sources.list.d/debian.sources): an older snapshot would downgrade packages
# of the base, and apt refuses that.
ARG SNAPSHOT=20261123T000000Z

# Non-root user of the container. The launch script passes the uid and gid of the host, so that
# the files written in the mounted repository belong to the host user.
ARG BUILDER_UID=1000
ARG BUILDER_GID=1000

ENV LANG=C.UTF-8
ARG DEBIAN_FRONTEND=noninteractive

# Frozen apt sources. HTTP is enough: integrity comes from the archive signatures, checked with
# the Debian keyring of the image. The Release files of a snapshot are expired by construction,
# hence Check-Valid-Until: no.
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
      > /etc/apt/sources.list.d/snapshot.sources \
 && printf '%s\n' \
      'Acquire::Retries "5";' \
      'APT::Install-Recommends "false";' \
      'APT::Install-Suggests "false";' \
      > /etc/apt/apt.conf.d/90builder

# Packages, by purpose. Versions come from the snapshot above.
#   GNU toolchain: the host requirements of Linux From Scratch.
#   Linux kernel: configuration, compilation, BTF (dwarves), modules, initramfs.
#   Sources: download and signature checks.
#   QEMU: qemu-system-x86_64, qemu-img, and the UEFI firmware (the target is UEFI, ADR 0001).
#   Disk images: GPT table (sfdisk, fdisk package) and EFI partition in FAT written without
#   root rights nor mounting (mtools). Added in lot Q-04 for the witness image of the boot bench.
RUN apt-get update \
 && apt-get install -y \
      bash binutils bison bzip2 coreutils diffutils file findutils gawk g++ gcc grep gzip \
      m4 make patch perl python3 sed tar texinfo xz-utils \
      bc cpio dwarves flex kmod libelf-dev libncurses-dev libssl-dev lz4 openssl rsync zstd \
      ca-certificates curl git gnupg wget \
      ovmf qemu-system-x86 qemu-utils \
      fdisk mtools \
 && rm -rf /var/lib/apt/lists/*

# Linux From Scratch requires /bin/sh to be bash. Debian links it to dash by default.
RUN ln -sf bash /usr/bin/sh

# -o accepts a uid or gid already taken in the image, for instance the users group (100).
RUN groupadd -o -g "${BUILDER_GID}" builder \
 && useradd -o -m -u "${BUILDER_UID}" -g "${BUILDER_GID}" -s /bin/bash builder \
 && mkdir -p /repo /cache/sources \
 && chown builder:builder /repo /cache/sources

ENV SOURCE_CACHE=/cache/sources
USER builder
WORKDIR /repo
CMD ["bash"]
