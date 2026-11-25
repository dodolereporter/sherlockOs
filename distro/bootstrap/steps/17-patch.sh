# LFS 13.1, section 6.13: patch.
# Sources: patch
enter patch
./configure --prefix=/usr --host="$LFS_TGT" --build="$(build-aux/config.guess)"
make
make DESTDIR="$LFS" install
