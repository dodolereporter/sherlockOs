#!/bin/sh
# Prints the commands that install the session for the login screen, from the release build of
# the repository given as argument. Runs nothing as root: the author copies the commands.
set -eu

repo="${1:-$PWD}"
dest=/usr/local/libexec/sherlockos
sessions=/usr/share/wayland-sessions
applications=/usr/local/share/applications
# The application icons, a hicolor theme that only the session sees (sherlockos-session puts
# this folder first in XDG_DATA_DIRS).
data=/usr/local/share/sherlockos
programs="compositor bar dock launcher sherlockos-apps sherlockos-clipboard notifications evidence sherlockos-lock control-room"

icons=/usr/local/share/icons
themes="sherlockos sherlockos-light"
for theme in $themes; do
	if [ ! -f "$repo/target/cursors/$theme/index.theme" ]; then
		echo "missing $repo/target/cursors/$theme: just cursors" >&2
		exit 1
	fi
done

if [ ! -d "$repo/target/wallpapers/first-clue" ]; then
	echo "missing $repo/target/wallpapers: just wallpapers" >&2
	exit 1
fi

for program in $programs; do
	if [ ! -x "$repo/target/release/$program" ]; then
		echo "missing $repo/target/release/$program: cargo build --release --bin $program" >&2
		exit 1
	fi
done

# The repository path is quoted in the commands: it may hold spaces.
q="'$(printf '%s' "$repo" | sed "s/'/'\\\\''/g")'"
cat <<EOF
# Run as root to install the SherlockOS session (shown by the login screen as "SherlockOS"):

sudo install -d $dest $applications $data/icons
sudo install -m 755 $(for p in $programs; do printf '%s/target/release/%s ' "$q" "$p"; done)$dest/
sudo install -m 755 $q/desktop/compositor/session/sherlockos-session $dest/
sudo install -m 644 $q/desktop/compositor/session/sherlockos.desktop $sessions/
sudo install -m 644 $q/desktop/evidence/evidence.desktop $applications/
sudo install -m 644 $q/desktop/control-room/control-room.desktop $applications/
# The wallpapers, which the compositor and Control Room look for in <data folder>/sherlockos:
sudo rm -rf $data/wallpapers
sudo cp -r $q/target/wallpapers $data/wallpapers
sudo cp -r $q/desktop/sherlock-ui/icons/hicolor $data/icons/
# The pictures of the lock screen, dark and light (lot D-25b):
sudo install -D -m 644 $q/assets/lock/lock-dark.jpg $data/lock/lock-dark.jpg
sudo install -D -m 644 $q/assets/lock/lock-light.jpg $data/lock/lock-light.jpg
# The PAM service of the lock screen (lot D-25a):
sudo install -m 644 $q/desktop/lock/pam/sherlockos-lock /etc/pam.d/sherlockos-lock
# Which portal backend serves the session (xdg-desktop-portal reads it from XDG_DATA_DIRS):
sudo install -D -m 644 $q/desktop/compositor/session/sherlockos-portals.conf $data/xdg-desktop-portal/sherlockos-portals.conf
# The cursor themes, their aliases kept as links:
sudo install -d $icons
sudo rm -rf $(for t in $themes; do printf '%s/%s ' "$icons" "$t"; done)
sudo cp -R $(for t in $themes; do printf '%s/target/cursors/%s ' "$q" "$t"; done)$icons/

# To remove it:
#   sudo rm -r $dest $data $sessions/sherlockos.desktop $applications/evidence.desktop $applications/control-room.desktop /etc/pam.d/sherlockos-lock
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

# What the session starts when it is there (see "Everyday services" in docs/compositor.md).
missing=""
have() {
	command -v "$1" >/dev/null 2>&1 || [ -x "$2" ]
}
have xdg-desktop-portal /usr/libexec/xdg-desktop-portal || missing="$missing xdg-desktop-portal"
have xdg-desktop-portal-gtk /usr/libexec/xdg-desktop-portal-gtk || missing="$missing xdg-desktop-portal-gtk"
have gnome-keyring-daemon /usr/bin/gnome-keyring-daemon || missing="$missing gnome-keyring"
have polkit-kde-authentication-agent-1 /usr/lib/x86_64-linux-gnu/libexec/polkit-kde-authentication-agent-1 ||
	missing="$missing polkit-kde-agent-1"
if [ -n "$missing" ]; then
	echo
	echo "# For the portals, the keyring and the polkit agent of the session:"
	echo "sudo apt install$missing"
fi
