# LFS 13.1, section 5.2: cross binutils, first so that gcc and glibc test the new linker.
# Sources: binutils
enter binutils
mkdir build
cd build
../configure --prefix="$LFS/tools" --with-sysroot="$LFS" --target="$LFS_TGT" \
    --disable-nls --enable-gprofng=no --disable-werror --enable-new-dtags \
    --enable-default-hash-style=gnu
make
make install
