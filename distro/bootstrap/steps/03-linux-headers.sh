# LFS 13.1, section 5.4: the kernel API headers, for glibc.
# Sources: linux
enter linux
make mrproper
make headers
find usr/include -type f ! -name '*.h' -delete
cp -r usr/include "$LFS/usr"
