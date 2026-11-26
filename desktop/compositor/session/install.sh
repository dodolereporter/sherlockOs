#!/bin/sh
# Prints the commands that install the session for the login screen, from the release build of
# the repository given as argument. Runs nothing as root: the author copies the commands.
set -eu

repo="${1:-$PWD}"
dest=/usr/local/libexec/nom-os
sessions=/usr/share/wayland-sessions
applications=/usr/local/share/applications
programs="compositor bar dock launcher notifications evidence"

icons=/usr/local/share/icons
themes="nom-os nom-os-light"
for theme in $themes; do
	if [ ! -f "$repo/target/cursors/$theme/index.theme" ]; then
		echo "missing $repo/target/cursors/$theme: just cursors" >&2
		exit 1
	fi
done

for program in $programs; do
	if [ ! -x "$repo/target/release/$program" ]; then
		echo "missing $repo/target/release/$program: cargo build --release -p $program" >&2
		exit 1
	fi
done

# The repository path is quoted in the commands: it may hold spaces.
q="'$(printf '%s' "$repo" | sed "s/'/'\\\\''/g")'"
cat <<EOF
# Run as root to install the NOM_OS session (shown by the login screen as "NOM_OS"):

sudo install -d $dest $applications
sudo install -m 755 $(for p in $programs; do printf '%s/target/release/%s ' "$q" "$p"; done)$dest/
sudo install -m 755 $q/desktop/compositor/session/sherlockos-session $dest/
sudo install -m 644 $q/desktop/compositor/session/sherlockos.desktop $sessions/
sudo install -m 644 $q/desktop/evidence/evidence.desktop $applications/
# The cursor themes, their aliases kept as links:
sudo install -d $icons
sudo rm -rf $(for t in $themes; do printf '%s/%s ' "$icons" "$t"; done)
sudo cp -R $(for t in $themes; do printf '%s/target/cursors/%s ' "$q" "$t"; done)$icons/

# To remove it:
#   sudo rm -r $dest $sessions/sherlockos.desktop $applications/evidence.desktop
#   sudo rm -r $(for t in $themes; do printf '%s/%s ' "$icons" "$t"; done)
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
