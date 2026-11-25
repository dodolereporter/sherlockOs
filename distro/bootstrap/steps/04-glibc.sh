# LFS 13.1, section 5.5: glibc, cross compiled into the sysroot.
# Sources: glibc glibc-fhs-patch glibc-fixes-patch
ln -sfn ../lib/ld-linux-x86-64.so.2 "$LFS/lib64/ld-linux-x86-64.so.2"
ln -sfn ../lib/ld-linux-x86-64.so.2 "$LFS/lib64/ld-lsb-x86-64.so.3"
enter glibc
# FHS paths instead of /var/db; upstream fixes (tanh on old x86_64, parallel install).
patch -Np1 -i "$(source_path glibc-fhs-patch)"
patch -Np1 -i "$(source_path glibc-fixes-patch)"
mkdir build
cd build
echo "rootsbindir=/usr/sbin" >configparms
../configure --prefix=/usr --host="$LFS_TGT" --build="$(../scripts/config.guess)" \
    --disable-nscd libc_cv_slibdir=/usr/lib --enable-kernel=5.10
make
make DESTDIR="$LFS" install
sed '/RTLDLIST=/s@/usr@@g' -i "$LFS/usr/bin/ldd"
# First sanity check of the book; check.sh does the full one at the end.
echo 'int main(){}' | "$LFS_TGT-gcc" -x c -
"$LFS_TGT-readelf" -l a.out | grep -F '[Requesting program interpreter: /lib64/ld-linux-x86-64.so.2]'
