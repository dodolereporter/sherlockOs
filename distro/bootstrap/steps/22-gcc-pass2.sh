# LFS 13.1, section 6.18: gcc for the target, built by the cross gcc of pass 1.
# Sources: gcc mpfr gmp mpc
enter gcc
unpack mpfr mpfr
unpack gmp gmp
unpack mpc mpc
sed -e '/m64=/s/lib64/lib/' -i.orig gcc/config/i386/t-linux64
mkdir build
cd build
../configure --build="$(../config.guess)" --host="$LFS_TGT" --target="$LFS_TGT" \
    --prefix=/usr --with-build-sysroot="$LFS" --enable-default-pie --enable-default-ssp \
    --disable-fixincludes --disable-nls --disable-multilib --disable-libatomic \
    --disable-libgomp --disable-libquadmath --disable-libsanitizer --disable-libssp \
    --disable-libvtv --enable-languages=c,c++ CXX_FOR_TARGET="$LFS_TGT-gcc -nostdinc++" \
    LDFLAGS_FOR_TARGET="-L$PWD/$LFS_TGT/libgcc" \
    target_configargs=gcc_cv_target_thread_file=posix
make
make DESTDIR="$LFS" install
ln -sfn gcc "$LFS/usr/bin/cc"
