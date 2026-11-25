# LFS 13.1, section 6.9: gawk, without its extras.
# Sources: gawk
enter gawk
sed -i 's/extras//' Makefile.in
./configure --prefix=/usr --host="$LFS_TGT" --build="$(build-aux/config.guess)"
make
make DESTDIR="$LFS" install
