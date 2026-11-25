# LFS 13.1, section 6.11: gzip.
# Sources: gzip
enter gzip
./configure --prefix=/usr --host="$LFS_TGT"
make
make DESTDIR="$LFS" install
