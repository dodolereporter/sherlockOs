/*
 * Only program of the initramfs of the witness image, started by the kernel as /init.
 *
 * It writes one line on the console, which the kernel has already opened as standard input and
 * output: that is the serial port, the last console of the command line. It copies the line to
 * the screen (/dev/tty0) so that the screenshot shows the same result. Then it waits forever:
 * the bench stops QEMU itself, after taking the screenshot.
 *
 * Compiled twice by build.sh: normally, it writes BOOT-OK; with -DWITNESS_SILENT, it writes
 * another line, to check that the bench fails cleanly.
 */
#include <fcntl.h>
#include <string.h>
#include <unistd.h>

#ifdef WITNESS_SILENT
static const char line[] = "witness: silent initramfs, no marker\n";
#else
static const char line[] = "BOOT-OK\n";
#endif

static void write_line(int fd)
{
    /* Nothing to do about a write error: without a console, nobody would read it. */
    if (write(fd, line, sizeof line - 1) < 0) {
        return;
    }
}

int main(void)
{
    write_line(STDOUT_FILENO);
    int screen = open("/dev/tty0", O_WRONLY | O_NOCTTY);
    if (screen >= 0) {
        write_line(screen);
        close(screen);
    }
    /* Init must never return: the kernel would panic. */
    for (;;) {
        pause();
    }
}
