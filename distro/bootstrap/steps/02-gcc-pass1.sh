# LFS 13.1, section 5.3: cross gcc, C and C++, without a C library yet.
# Sources: gcc mpfr gmp mpc glibc
enter gcc
unpack mpfr mpfr
unpack gmp gmp
unpack mpc mpc
# 64-bit libraries go to lib, not lib64.
sed -e '/m64=/s/lib64/lib/' -i.orig gcc/config/i386/t-linux64
mkdir build
cd build
../configure --target="$LFS_TGT" --prefix="$LFS/tools" --with-glibc-version="$(version glibc)" \
    --with-sysroot="$LFS" --with-newlib --without-headers --enable-default-pie \
    --enable-default-ssp --disable-fixincludes --disable-nls --disable-shared \
    --disable-multilib --disable-threads --disable-libatomic --disable-libgomp \
    --disable-libquadmath --disable-libssp --disable-libvtv --disable-libstdcxx \
    --enable-languages=c,c++
make
make install
# The limits.h installed without a C library is partial: write the full one.
cat ../gcc/{limitx,glimits,limity}.h >"$("$LFS_TGT-gcc" -print-file-name=include)/limits.h"
