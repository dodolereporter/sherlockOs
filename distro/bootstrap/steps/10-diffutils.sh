# LFS 13.1, section 6.6: diffutils. The strcasecmp check cannot run when cross compiling.
# Sources: diffutils
enter diffutils
./configure --prefix=/usr --host="$LFS_TGT" gl_cv_func_strcasecmp_works=yes \
    --build="$(./build-aux/config.guess)"
make
make DESTDIR="$LFS" install
