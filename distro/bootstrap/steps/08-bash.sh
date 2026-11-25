# LFS 13.1, section 6.4: bash.
# Sources: bash
enter bash
./configure --prefix=/usr --build="$(sh support/config.guess)" --host="$LFS_TGT" \
    --without-bash-malloc --docdir="/usr/share/doc/bash-$(version bash)"
make
make DESTDIR="$LFS" install
ln -sfn bash "$LFS/bin/sh"
