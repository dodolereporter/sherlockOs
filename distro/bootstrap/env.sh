# Environment of every build step, after build.sh has emptied the inherited one (env -i).
# Same settings as the lfs user of Linux From Scratch (section 4.4). Sourced, never run.
# Part of the fingerprint of every step: changing a line here rebuilds everything.

set +h
umask 022

# The future root of the system. The cross tools go to $LFS/tools, the rest to its final place.
export LFS="$TOOLCHAIN_OUTPUT/sysroot"
# Target triplet. The vendor field only tells this toolchain apart from the host's: it holds the
# placeholder of the OS name, to change together with it.
export LFS_TGT=x86_64-nomos-linux-gnu
export LC_ALL=POSIX
# The new tools come first. /bin is a link to /usr/bin in the container.
export PATH="$LFS/tools/bin:/usr/bin"
# Keeps configure away from the config.site of the host.
export CONFIG_SITE="$LFS/usr/share/config.site"
export MAKEFLAGS="-j$TOOLCHAIN_JOBS"
