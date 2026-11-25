# LFS 13.1, section 6.10: grep.
# Sources: grep
enter grep
./configure --prefix=/usr --host="$LFS_TGT" --build="$(./build-aux/config.guess)"
make
make DESTDIR="$LFS" install
