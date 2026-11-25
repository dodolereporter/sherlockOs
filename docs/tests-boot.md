# Banc de boot QEMU

Chaque lot de la piste système doit prouver que son image boote. Le banc démarre une image disque
dans QEMU, sans écran, lit le port série et dit si la ligne `BOOT-OK` est sortie en moins de
90 secondes. Il prend aussi une capture de l'écran virtuel. Tout tourne dans le conteneur de
build (voir `docs/build.md`) : le poste ne fournit que Docker et `just`.

## Commandes

| Commande | Ce qu'elle fait |
| --- | --- |
| `just test-boot` | Construit l'image témoin, puis la boote. Réussit si `BOOT-OK` sort |
| `just test-boot <image>` | Boote l'image donnée, sans rien construire |
| `just temoin` | Construit l'image témoin seule : `distro/tests/sortie/temoin.img` |
| `just temoin muet` | Construit la variante qui n'écrit pas `BOOT-OK` : `temoin-muet.img` |
| `just tester-banc` | Vérifie que le banc échoue proprement (voir « Tests du banc ») |

L'image passée à `just test-boot` doit être dans le dépôt, par exemple sous
`distro/tests/sortie/` : seul le dépôt est monté dans le conteneur.

## Ce que fait `distro/tests/boot.sh`

`boot.sh <image>` tourne dans le conteneur. Il :

1. lit le format de l'image (`raw`, `qcow2`…) avec `qemu-img info` ;
2. lance `qemu-system-x86_64` en machine `q35`, avec le micrologiciel UEFI OVMF (la cible est UEFI,
   ADR 0001), l'image en disque virtio, une carte graphique `std` sans fenêtre (`-display none`)
   et le port série écrit dans un fichier ;
3. regarde le port série toutes les demi-secondes, jusqu'à la ligne `BOOT-OK`, l'arrêt de QEMU ou
   la fin du délai ;
4. prend une capture de l'écran par QMP (`screendump`, en PNG), puis arrête QEMU.

| Résultat | Code | Affichage |
| --- | --- | --- |
| Une ligne qui vaut exactement `BOOT-OK` sort avant le délai | 0 | Durée du boot, chemin du journal |
| Pas de `BOOT-OK` au bout du délai | 1 | `ÉCHEC`, puis les 50 dernières lignes du port série |
| QEMU s'arrête avant (plantage, redémarrage) | 1 | Pareil, avec le code de sortie de QEMU |
| Image absente, délai invalide, outil manquant | 2 | Le problème, sans lancer QEMU |

Quelques choix :

- **`-snapshot`** : ce que le système démarré écrit sur son disque est jeté. L'image testée ne
  change pas.
- **`-no-reboot`** : un noyau qui plante et redémarre arrête QEMU, au lieu de boucler jusqu'au délai.
- **KVM** quand `/dev/kvm` est ouvrable, sinon émulation (`-accel tcg`). Le banc affiche lequel.
- **Variables UEFI** : copiées dans un dossier temporaire à chaque lancement, jamais gardées.
- Les fins de ligne `\r` du port série et les séquences d'échappement d'OVMF sont retirées des
  50 lignes affichées. Le journal complet garde tout.

| Variable | Par défaut | Rôle |
| --- | --- | --- |
| `BOOT_DELAI` | `90` | Secondes accordées au boot |
| `BOOT_SORTIE` | `distro/tests/sortie/boot-<nom de l'image>` | Dossier du journal et de la capture |

Exemple : `BOOT_DELAI=30 just test-boot distro/tests/sortie/temoin.img`.

Chaque lancement écrit dans son dossier de sortie :

| Fichier | Contenu |
| --- | --- |
| `serie.log` | Tout le port série : OVMF, noyau, `/init` |
| `capture.png` | L'écran virtuel au moment du verdict, réussite ou échec |

Une image qui veut passer le banc écrit `BOOT-OK`, seul sur sa ligne, sur le premier port série
(`ttyS0`, 115200 bauds) une fois démarrée. La vraie image (lots S-11 et S-15) devra le faire.

## L'image témoin

Elle sert à prouver que le banc marche, tant que la vraie image n'existe pas. Fichiers dans
`distro/tests/temoin/`, construits par `construire.sh`, dans le conteneur :

| Élément | Choix |
| --- | --- |
| Noyau | Linux 7.3.6, stable de kernel.org au 25 novembre 2026, épinglé avec son empreinte SHA-256 |
| Configuration | `make tinyconfig`, plus les options de `noyau.config` : ELF, initramfs, UEFI, port série, console sur le tampon d'image UEFI |
| Initramfs | Intégré au noyau. Un seul binaire, `/init`, compilé en statique depuis `init.c` |
| Disque | 64 Mio, table GPT, une partition EFI en FAT32, le noyau dans `\EFI\BOOT\BOOTX64.EFI` |
| Taille du noyau | Environ 1,6 Mo, initramfs compris |

`/init` écrit `BOOT-OK` sur le port série et sur l'écran, puis attend. La variante muette écrit
une autre ligne à la place. Le micrologiciel lance le noyau directement, sans chargeur : le noyau
est aussi une application UEFI (`CONFIG_EFI_STUB`), avec sa ligne de commande intégrée.

`construire.sh` vérifie que chaque option de `noyau.config` se retrouve dans le `.config` final, et
s'arrête sinon : une option que `make olddefconfig` aurait retirée en silence casserait le boot.

L'archive du noyau est téléchargée une fois dans le cache des sources (`CACHE_SOURCES`, voir
`docs/build.md`) et son empreinte est vérifiée à chaque construction. Une archive fausse est
supprimée. Tout le reste va dans `distro/tests/sortie/`, ignoré par git : sources extraites
(environ 1,8 Go), dossier de compilation, images, journaux, captures. Pour tout effacer, supprime
ce dossier.

La première construction extrait les sources et compile le noyau, puis seules les étapes changées
sont refaites. `TEMOIN_JOBS` fixe le nombre de compilations en parallèle, tous les processeurs
par défaut, par exemple `TEMOIN_JOBS=4 just test-boot`.

### Monter la version du noyau

1. Relève la version sur kernel.org et télécharge `sha256sums.asc` du dossier `v<majeure>.x`.
2. Vérifie sa signature, faite par la clé « Kernel.org checksum autosigner »
   (`B8868C80BA62A1FFFAF5FDA9632D3A06589DA6B1`), par exemple
   `gpg --auto-key-locate clear,wkd --locate-keys autosigner@kernel.org` puis `gpg --verify sha256sums.asc`.
3. Recopie la version et l'empreinte du `.tar.xz` en tête de `construire.sh`, puis lance
   `just test-boot` et `just tester-banc`.

## Tests du banc

`just test-boot` est le cas qui réussit. `just tester-banc` construit la variante muette et vérifie,
avec `distro/tests/tester-banc.sh`, que :

- le banc rend 1 au bout des 90 secondes, avec le message du délai ;
- il affiche au plus 50 lignes du port série, dont la ligne écrite par `/init` à la place de `BOOT-OK` ;
- la capture PNG est prise malgré l'échec ;
- une image absente rend 2 sans lancer QEMU.

Ce test dure donc un peu plus de 90 secondes.

## Dans la CI

Le workflow `.github/workflows/ci-boot.yml`, tâche `test-boot`, tourne sur chaque demande de fusion
et sur `main`, comme la CI Rust (`docs/ci.md`). Il :

1. rend `/dev/kvm` ouvrable par l'utilisateur de la machine, pour que QEMU en profite ;
2. restaure l'image de build depuis le cache, ou la construit ;
3. lance `just tester-builder`, `just test-boot` puis `just tester-banc` ;
4. publie les captures et les journaux série dans l'artefact `captures-boot`, même en cas d'échec.

**Cache de l'image de build.** Construire l'image prend quelques minutes et dépend de
`snapshot.debian.org`. L'image est donc gardée dans le cache de GitHub Actions, exportée par
`docker save` et compressée en zstd. La clé contient l'empreinte du `Dockerfile` et l'uid de la
machine, les deux choses qui font l'empreinte de `builder.sh` : une image restaurée n'est pas
reconstruite. Seules les exécutions sur `main` enregistrent le cache. Les demandes de fusion le
relisent, et une demande qui change le `Dockerfile` reconstruit l'image.

L'archive du noyau n'est pas mise en cache : elle vient du CDN de kernel.org à chaque exécution,
et son empreinte est vérifiée.

Pour lire un échec : le journal de l'étape `just test-boot` montre les 50 dernières lignes du port
série, et l'artefact `captures-boot` donne la capture de l'écran et le journal complet.
