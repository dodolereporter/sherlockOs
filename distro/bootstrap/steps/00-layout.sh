# LFS 13.1, section 4.2: limited directory layout of the future root. No sources.
# /bin, /lib and /sbin are links to /usr, /lib64 only holds the dynamic loader links.
mkdir -p "$LFS"/{etc,var,lib64,tools} "$LFS"/usr/{bin,lib,sbin}
for dir in bin lib sbin; do
    ln -sfn "usr/$dir" "$LFS/$dir"
done
