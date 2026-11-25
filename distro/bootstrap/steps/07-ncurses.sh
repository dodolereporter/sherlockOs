# LFS 13.1, section 6.3: ncurses. Its tic runs on the build machine, so it is built first.
# Sources: ncurses
enter ncurses
mkdir build
pushd build
../configure --prefix="$LFS/tools" AWK=gawk
make -C include
make -C progs tic
install progs/tic "$LFS/tools/bin"
popd
./configure --prefix=/usr --host="$LFS_TGT" --build="$(./config.guess)" \
    --mandir=/usr/share/man --with-manpage-format=normal --with-shared --without-normal \
    --with-cxx-shared --without-debug --without-ada --disable-stripping AWK=gawk
make
make DESTDIR="$LFS" install
ln -sfn libncursesw.so "$LFS/usr/lib/libncurses.so"
# Always the wide character structures, those of libncursesw.
sed -e 's/^#if.*XOPEN.*$/#if 1/' -i "$LFS/usr/include/curses.h"
