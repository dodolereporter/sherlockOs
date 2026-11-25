/*
 * Seul programme de l'initramfs de l'image témoin, lancé par le noyau comme /init.
 *
 * Il écrit une ligne sur la console, que le noyau a déjà ouverte en entrées et sorties
 * standard : c'est le port série, la dernière console de la ligne de commande. Il la recopie
 * sur l'écran (/dev/tty0) pour que la capture montre le même résultat. Puis il attend sans
 * fin : le banc arrête QEMU lui-même, après avoir pris la capture de l'écran.
 *
 * Compilé deux fois par construire.sh : normalement, il écrit BOOT-OK ; avec -DTEMOIN_MUET,
 * il écrit une autre ligne, pour vérifier que le banc échoue proprement.
 */
#include <fcntl.h>
#include <string.h>
#include <unistd.h>

#ifdef TEMOIN_MUET
static const char ligne[] = "temoin : initramfs muet, pas de marqueur\n";
#else
static const char ligne[] = "BOOT-OK\n";
#endif

static void ecrire(int fd)
{
    /* Rien à faire d'une erreur d'écriture : sans console, personne ne la lirait. */
    if (write(fd, ligne, sizeof ligne - 1) < 0) {
        return;
    }
}

int main(void)
{
    ecrire(STDOUT_FILENO);
    int ecran = open("/dev/tty0", O_WRONLY | O_NOCTTY);
    if (ecran >= 0) {
        ecrire(ecran);
        close(ecran);
    }
    /* Init ne doit jamais rendre la main : le noyau paniquerait. */
    for (;;) {
        pause();
    }
}
