# LFS 13.1, section 6.12: make.
# Sources: make
enter make
./configure --prefix=/usr --host="$LFS_TGT" --build="$(build-aux/config.guess)"
make
make DESTDIR="$LFS" install
