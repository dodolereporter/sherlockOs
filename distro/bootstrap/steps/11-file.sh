# LFS 13.1, section 6.7: file. Its magic database is compiled by a file of the same version,
# built first for the build machine.
# Sources: file
enter file
mkdir build
pushd build
../configure --disable-bzlib --disable-libseccomp --disable-xzlib --disable-zlib
make
popd
./configure --prefix=/usr --host="$LFS_TGT" --build="$(./config.guess)"
make FILE_COMPILE="$(pwd)/build/src/file"
make DESTDIR="$LFS" install
rm -f "$LFS/usr/lib/libmagic.la"
