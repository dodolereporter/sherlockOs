# LFS 13.1, section 6.14: sed.
# Sources: sed
enter sed
./configure --prefix=/usr --host="$LFS_TGT" --build="$(./build-aux/config.guess)"
make
make DESTDIR="$LFS" install
