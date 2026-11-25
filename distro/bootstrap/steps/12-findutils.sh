# LFS 13.1, section 6.8: findutils.
# Sources: findutils
enter findutils
./configure --prefix=/usr --localstatedir=/var/lib/locate --host="$LFS_TGT" \
    --build="$(build-aux/config.guess)"
make
make DESTDIR="$LFS" install
