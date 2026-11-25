# LFS 13.1, section 6.5: coreutils, with hostname for the perl test suite.
# Sources: coreutils
enter coreutils
./configure --prefix=/usr --host="$LFS_TGT" --build="$(build-aux/config.guess)" \
    --enable-install-program=hostname
make
make DESTDIR="$LFS" install
# Some programs expect chroot in sbin.
mv "$LFS/usr/bin/chroot" "$LFS/usr/sbin"
mkdir -p "$LFS/usr/share/man/man8"
mv "$LFS/usr/share/man/man1/chroot.1" "$LFS/usr/share/man/man8/chroot.8"
sed -i 's/"1"/"8"/' "$LFS/usr/share/man/man8/chroot.8"
