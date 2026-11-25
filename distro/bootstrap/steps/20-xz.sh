# LFS 13.1, section 6.16: xz.
# Sources: xz
enter xz
./configure --prefix=/usr --host="$LFS_TGT" --build="$(build-aux/config.guess)" \
    --disable-static --docdir="/usr/share/doc/xz-$(version xz)"
make
make DESTDIR="$LFS" install
rm -f "$LFS/usr/lib/liblzma.la"
