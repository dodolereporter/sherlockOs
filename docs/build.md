# Building the distribution

The distribution is built from source, always in the build container described by
`distro/builder/Dockerfile`. The host only provides Docker: compilers, kernel tools and QEMU come
from the image. A build that works on your machine therefore works on someone else's and in the CI.

## Host prerequisites

| Tool | Why |
| --- | --- |
| Docker | Builds the image and runs the container. Your user must be able to call `docker` without `sudo` |
| `just` | Runs the recipes of the `justfile` |
| Linux x86_64 | The target is x86_64 (ADR 0001). The launch script assumes bash and the basic GNU tools |
| Network | Building the image downloads its packages from `snapshot.debian.org` |
| Disk | About 930 MB for the image, measured on its first build, plus the source cache |

KVM is optional. If `/dev/kvm` exists and your user can open it, it is passed to the container and
QEMU uses it. Otherwise QEMU emulates the processor, much more slowly.

## Commands

| Command | What it does |
| --- | --- |
| `just builder` | Builds the `sherlockos-builder:local` image |
| `just in-builder <command>` | Runs the command in the container, then removes it |
| `just in-builder` | Opens a bash in the container |
| `just test-builder` | Tests the container, in a few seconds once the image is built (see "Tests") |
| `just test-builder-repro` | Checks that two builds install the same versions. Slow (see "Tests") |

Examples:

```sh
just builder
just in-builder gcc --version
just in-builder qemu-system-x86_64 --version
just in-builder bash -c 'cd "$SOURCE_CACHE" && ls'
```

`just in-builder` rebuilds the image by itself when it is missing or when the `Dockerfile` has
changed since the last build. It compares a fingerprint set as a label on the image.

## Inside the container

| Item | Value |
| --- | --- |
| User | `builder`, with the uid and gid of your user. Never root |
| Repository | Mounted read-write on `/repo`, which is also the current folder |
| Source cache | Mounted on `/cache/sources`, path given by the `SOURCE_CACHE` variable |
| `/bin/sh` | bash, as Linux From Scratch requires |
| Container name | `sherlockos-builder-<process id>`, removed on exit |

Files written in `/repo` belong to your user on the host. If your host runs as root, which happens
on some CI, the container user takes uid 1000.

Everything written outside `/repo` and `/cache/sources` disappears with the container.

In a git worktree, the `.git` folder of the main repository is not mounted: `git` does not work in
the container. The build commands do not need it.

## Source cache

Downloaded source archives live outside the git repository, so that they are not downloaded again
on every build and never committed.

| | Path |
| --- | --- |
| Default | `${XDG_CACHE_HOME:-$HOME/.cache}/sherlockos/sources`, usually `~/.cache/sherlockos/sources` |
| To change it | Export `SOURCE_CACHE`, for instance `SOURCE_CACHE=/data/sources just in-builder …` |
| In the container | `/cache/sources`, also in the `SOURCE_CACHE` variable |

The folder is created on the first run. A cache inside the repository is refused. To empty it,
simply delete the folder on the host.

## Pinned versions

Two builds of the image install exactly the same packages, at the same versions:

- the base image `debian:trixie-slim` is pinned by its `sha256` digest;
- apt reads `snapshot.debian.org` at a frozen point in time (`ARG SNAPSHOT` of the `Dockerfile`),
  not the live Debian mirror. That point is the one the base image was built from.

`just test-builder-repro` checks it (see "Tests").

To move to newer versions, read the new digest of `debian:trixie-slim`, read the snapshot time in
its `/etc/apt/sources.list.d/debian.sources`, change both in the `Dockerfile`, rebuild, then run
`just test-builder` and `just test-builder-repro` again. A snapshot older than the base image makes
apt fail: it would have to downgrade packages.

## Tests

Both recipes call `distro/builder/test.sh`. They print one `ok` or `FAILED` line per check and exit
with an error as soon as a check fails.

`just test-builder` builds the image if it is missing, then checks that:

- `gcc --version` and `qemu-system-x86_64 --version` answer in the container;
- the uid in the container is not 0;
- `/bin/sh` points to bash;
- a `SOURCE_CACHE` inside the repository is refused without anything being created, whether the
  path is absolute, relative, the repository root or a detour through `..`;
- the exit code of the command is returned, by `builder.sh` as by `just in-builder`: `false` gives
  1 and `bash -c 'exit 7'` gives 7;
- arguments pass intact, spaces and apostrophes included.

`just test-builder-repro` builds the image twice without cache, under the temporary tag
`sherlockos-builder:repro-<pid>`, and compares the `dpkg-query -W` list of both builds. Count about
4 min 30, measured on the author's machine, and two full downloads of the packages. The test image
is removed at the end, even on failure, and the base image too if the test had to download it.
`sherlockos-builder:local` is not touched.

This test goes through the legacy Docker builder (`DOCKER_BUILDKIT=0`), which prints a deprecation
warning. Unlike BuildKit, it keeps no build cache: removing the test image frees everything it
used. The installed versions only depend on the `Dockerfile`, not on the builder.

## Variables

| Variable | Default | Role |
| --- | --- | --- |
| `SOURCE_CACHE` | `~/.cache/sherlockos/sources` | Folder of the source cache on the host |
| `BUILDER_IMAGE` | `sherlockos-builder:local` | Name of the image built and run |
