# LFS 13.1, section 6.17: binutils for the target, with shared libraries.
# Sources: binutils
enter binutils
# Keeps libtool from linking the tools against libraries of the build machine.
sed '6031s/$add_dir//' -i ltmain.sh
mkdir build
cd build
../configure --prefix=/usr --build="$(../config.guess)" --host="$LFS_TGT" --disable-nls \
    --enable-shared --enable-gprofng=no --disable-werror --enable-64-bit-bfd \
    --enable-new-dtags --enable-default-hash-style=gnu
make
make DESTDIR="$LFS" install
rm -f "$LFS"/usr/lib/lib{bfd,ctf,ctf-nobfd,opcodes,sframe}.{a,la}
