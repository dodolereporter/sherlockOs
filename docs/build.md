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
| `just tester-builder` | Teste le conteneur, en quelques secondes une fois l'image construite (voir « Tests ») |
| `just tester-builder-repro` | Vérifie que deux constructions installent les mêmes versions. Lent (voir « Tests ») |

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

`just tester-builder-repro` le vérifie (voir « Tests »).

Pour monter de version, relève la nouvelle empreinte de `debian:trixie-slim`, lis l'instant de
snapshot dans son `/etc/apt/sources.list.d/debian.sources`, change les deux dans le `Dockerfile`,
reconstruis, puis relance `just tester-builder` et `just tester-builder-repro`. Un instantané plus ancien que l'image
de base fait échouer apt : il faudrait rétrograder des paquets.

## Tests

Les deux recettes appellent `distro/builder/tester.sh`. Elles affichent une ligne `ok` ou `ÉCHEC`
par vérification et sortent en erreur dès qu'une vérification échoue.

`just tester-builder` construit l'image si elle manque, puis vérifie que :

- `gcc --version` et `qemu-system-x86_64 --version` répondent dans le conteneur ;
- l'uid dans le conteneur n'est pas 0 ;
- `/bin/sh` pointe vers bash ;
- un `CACHE_SOURCES` placé dans le dépôt est refusé sans que rien soit créé, que le chemin soit
  absolu, relatif, la racine du dépôt ou un détour par `..` ;
- le code de sortie de la commande est rendu, par `builder.sh` comme par `just dans-builder` :
  `false` donne 1 et `bash -c 'exit 7'` donne 7 ;
- les arguments passent intacts, espaces et apostrophes compris.

`just tester-builder-repro` construit deux fois l'image sans cache, sous l'étiquette temporaire
`sherlockos-builder:repro-<pid>`, et compare la liste `dpkg-query -W` des deux constructions.
Compte environ 4 min 30, mesuré sur le poste de l'auteur, et deux téléchargements complets des paquets. L'image d'essai est
supprimée à la fin, même en cas d'échec, et l'image de base aussi si le test a dû la télécharger.
`sherlockos-builder:local` n'est pas touchée.

Ce test passe par le builder historique de Docker (`DOCKER_BUILDKIT=0`), qui affiche un
avertissement d'obsolescence. Contrairement à BuildKit, il ne garde pas de cache de construction :
supprimer l'image d'essai libère tout ce qu'elle occupait. Les versions installées ne dépendent
que du `Dockerfile`, pas du builder.

## Variables

| Variable | Par défaut | Rôle |
| --- | --- | --- |
| `CACHE_SOURCES` | `~/.cache/sherlockos/sources` | Dossier du cache des sources sur le poste |
| `BUILDER_IMAGE` | `sherlockos-builder:local` | Nom de l'image construite et lancée |
