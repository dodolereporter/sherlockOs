#!/bin/sh
# Prints the commands that install the session for the login screen, from the release build of
# the repository given as argument. Runs nothing as root: the author copies the commands.
set -eu

repo="${1:-$PWD}"
dest=/usr/local/libexec/nom-os
sessions=/usr/share/wayland-sessions

for program in compositor bar dock; do
	if [ ! -x "$repo/target/release/$program" ]; then
		echo "missing $repo/target/release/$program: cargo build --release -p $program" >&2
		exit 1
	fi
done

# The repository path is quoted in the commands: it may hold spaces.
q="'$(printf '%s' "$repo" | sed "s/'/'\\\\''/g")'"
cat <<EOF
# Run as root to install the NOM_OS session (shown by the login screen as "NOM_OS"):

sudo install -d $dest
sudo install -m 755 $q/target/release/compositor $q/target/release/bar $q/target/release/dock $dest/
sudo install -m 755 $q/desktop/compositor/session/sherlockos-session $dest/
sudo install -m 644 $q/desktop/compositor/session/sherlockos.desktop $sessions/

# To remove it:
#   sudo rm -r $dest $sessions/sherlockos.desktop
EOF

# The compositor needs libseat at run time (seatd or logind behind it). ldconfig is in the
# administrator's PATH only.
ldconfig=$(command -v ldconfig || true)
for candidate in /sbin/ldconfig /usr/sbin/ldconfig; do
	if [ -z "$ldconfig" ] && [ -x "$candidate" ]; then
		ldconfig=$candidate
	fi
done
if [ -n "$ldconfig" ] && ! "$ldconfig" -p | grep -q 'libseat\.so\.1 '; then
	echo
	echo "# libseat is missing:"
	echo "sudo apt install libseat1"
fi
