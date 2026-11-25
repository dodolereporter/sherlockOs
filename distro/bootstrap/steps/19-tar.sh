# LFS 13.1, section 6.15: tar.
# Sources: tar
enter tar
./configure --prefix=/usr --host="$LFS_TGT" --build="$(build-aux/config.guess)"
make
make DESTDIR="$LFS" install
