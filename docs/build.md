# Construire la distribution

La distribution se construit depuis les sources, toujours dans le conteneur de build décrit par
`distro/builder/Dockerfile`. Le poste ne fournit que Docker : compilateurs, outils du noyau et QEMU
viennent de l'image. Un build qui marche chez toi marche donc chez un autre et dans la CI.

## Prérequis du poste

| Outil | Pourquoi |
| --- | --- |
| Docker | Construit l'image et lance le conteneur. Ton utilisateur doit pouvoir appeler `docker` sans `sudo` |
| `just` | Lance les recettes du `justfile` |
| Linux x86_64 | La cible est x86_64 (ADR 0001). Le script de lancement suppose bash et les outils GNU de base |
| Réseau | La construction de l'image télécharge ses paquets depuis `snapshot.debian.org` |
| Disque | Environ 930 Mo pour l'image, mesuré à sa première construction, plus le cache des sources |

KVM est facultatif. Si `/dev/kvm` existe et que ton utilisateur peut l'ouvrir, il est passé au
conteneur et QEMU s'en sert. Sinon QEMU émule le processeur, beaucoup plus lentement.

## Commandes

| Commande | Ce qu'elle fait |
| --- | --- |
| `just builder` | Construit l'image `sherlockos-builder:local` |
| `just dans-builder <commande>` | Lance la commande dans le conteneur, puis le supprime |
| `just dans-builder` | Ouvre un bash dans le conteneur |

Exemples :

```sh
just builder
just dans-builder gcc --version
just dans-builder qemu-system-x86_64 --version
just dans-builder bash -c 'cd "$CACHE_SOURCES" && ls'
```

`just dans-builder` reconstruit l'image tout seul quand elle manque ou quand le `Dockerfile` a
changé depuis la dernière construction. Il compare une empreinte posée en étiquette sur l'image.

## Dans le conteneur

| Élément | Valeur |
| --- | --- |
| Utilisateur | `builder`, avec l'uid et le gid de ton utilisateur. Jamais root |
| Dépôt | Monté en lecture et écriture dans `/depot`, qui est aussi le dossier courant |
| Cache des sources | Monté dans `/cache/sources`, chemin donné par la variable `CACHE_SOURCES` |
| `/bin/sh` | bash, comme l'exige Linux From Scratch |
| Nom du conteneur | `sherlockos-builder-<numéro du processus>`, supprimé à la sortie |

Les fichiers écrits dans `/depot` appartiennent à ton utilisateur sur le poste. Si ton poste tourne
en root, ce qui arrive sur certaines CI, l'utilisateur du conteneur prend l'uid 1000.

Tout ce qui est écrit hors de `/depot` et de `/cache/sources` disparaît avec le conteneur.

Dans un worktree git, le dossier `.git` du dépôt principal n'est pas monté : `git` ne marche pas
dans le conteneur. Les commandes de build n'en ont pas besoin.

## Cache des sources

Les archives de sources téléchargées vivent hors du dépôt git, pour ne pas les retélécharger à
chaque build et ne jamais les committer.

| | Chemin |
| --- | --- |
| Par défaut | `${XDG_CACHE_HOME:-$HOME/.cache}/sherlockos/sources`, soit le plus souvent `~/.cache/sherlockos/sources` |
| Pour le changer | Exporte `CACHE_SOURCES`, par exemple `CACHE_SOURCES=/data/sources just dans-builder …` |
| Dans le conteneur | `/cache/sources`, aussi dans la variable `CACHE_SOURCES` |

Le dossier est créé au premier lancement. Un cache placé dans le dépôt est refusé. Pour le vider,
supprime simplement le dossier sur le poste.

## Versions figées

Deux constructions de l'image installent exactement les mêmes paquets, aux mêmes versions :

- l'image de base `debian:trixie-slim` est épinglée par son empreinte `sha256` ;
- apt lit `snapshot.debian.org` à un instant figé (`ARG SNAPSHOT` du `Dockerfile`), pas le miroir
  courant de Debian. Cet instant est celui à partir duquel l'image de base a été construite.

Pour vérifier, compare la liste des paquets de deux constructions sans cache :

```sh
docker build --no-cache -t sherlockos-builder:essai distro/builder
docker run --rm sherlockos-builder:essai dpkg-query -W > /tmp/paquets-1.txt
docker build --no-cache -t sherlockos-builder:essai distro/builder
docker run --rm sherlockos-builder:essai dpkg-query -W > /tmp/paquets-2.txt
diff /tmp/paquets-1.txt /tmp/paquets-2.txt && docker rmi sherlockos-builder:essai
```

Pour monter de version, relève la nouvelle empreinte de `debian:trixie-slim`, lis l'instant de
snapshot dans son `/etc/apt/sources.list.d/debian.sources`, change les deux dans le `Dockerfile`,
reconstruis, puis relance `just dans-builder gcc --version`. Un instantané plus ancien que l'image
de base fait échouer apt : il faudrait rétrograder des paquets.

## Variables

| Variable | Par défaut | Rôle |
| --- | --- | --- |
| `CACHE_SOURCES` | `~/.cache/sherlockos/sources` | Dossier du cache des sources sur le poste |
| `BUILDER_IMAGE` | `sherlockos-builder:local` | Nom de l'image construite et lancée |
