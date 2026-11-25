# LFS 13.1, section 5.6: libstdc++ for the target, now that glibc is there.
# Sources: gcc
enter gcc
mkdir build
cd build
../libstdc++-v3/configure --host="$LFS_TGT" --build="$(../config.guess)" CXX="$LFS_TGT-gcc" \
    --prefix=/usr --disable-multilib --disable-nls --disable-libstdcxx-pch \
    --with-gxx-include-dir="/tools/$LFS_TGT/include/c++/$(version gcc)"
make
make DESTDIR="$LFS" install
# Libtool archives are harmful when cross compiling.
rm -f "$LFS"/usr/lib/lib{stdc++{,exp,fs},supc++}.la
