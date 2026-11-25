# LFS 13.1, section 6.2: M4.
# Sources: m4
# gnulib cannot test these glibc 2.44 functions when cross compiling.
cat >"$LFS/usr/share/config.site" <<END
ac_cv_func_posix_spawn_file_actions_addchdir=yes
ac_cv_func_posix_spawn_file_actions_addfchdir=yes
END
enter m4
./configure --prefix=/usr --host="$LFS_TGT" --build="$(build-aux/config.guess)"
make
make DESTDIR="$LFS" install
