#!/bin/sh
# Prints the commands that make the SherlockOS login screen (greetd, ADR 0026) the display manager
# of the machine, from the release build of the repository given as argument, then the commands
# that go back to SDDM. Runs nothing as root: the author copies the commands.
set -eu

repo="${1:-$PWD}"
mascot="${2:-${XDG_DATA_HOME:-$HOME/.local/share}/sherlockos/mascot/sherlock.glb}"
dest=/usr/local/libexec/sherlockos
data=/usr/local/share/sherlockos
memory=/var/cache/sherlockos-greeter
# The user the Debian package of greetd creates and runs its greeter as.
user=_greetd
programs="compositor sherlockos-greeter"

# The release build of CARGO_TARGET_DIR, where `just install-greeter` has just built it; every
# program up to date with the sources of this repository, or nothing is printed.
. "$(dirname "$0")/../compositor/session/built.sh"
release=$(release_dir "$repo")
check_built "$repo" "$release" $programs
if [ ! -f "$mascot" ]; then
	echo "note: no model of Sherlock at $mascot (just mascot): the screen shows his still picture" >&2
	mascot=""
fi

# The repository path is quoted in the commands: it may hold spaces.
quote() {
	printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}
q=$(quote "$repo")
command="$dest/compositor --greeter -- $dest/sherlockos-greeter"

cat <<EOF
# Run as root to make the SherlockOS login screen the display manager. The session itself
# (« Bureau ») comes from just install-session: run its commands first.

# 1. greetd (Debian's package creates the $user user, and enables nothing while SDDM holds
#    display-manager.service):
sudo apt install greetd

# 2. The programs, and what the screen reads, where $user can read them:
sudo install -d $dest $data/lock $data/mascot
sudo install -m 755 $(for p in $programs; do printf '%s/%s ' "$(quote "$release")" "$p"; done)$dest/
sudo install -m 644 $q/assets/lock/lock-dark.jpg $q/assets/lock/lock-light.jpg $data/lock/
EOF
if [ -n "$mascot" ]; then
	echo "sudo install -m 644 $(quote "$mascot") $data/mascot/sherlock.glb"
fi
cat <<EOF

# 3. The folder where the screen remembers the last user, writable by $user only:
sudo install -d -o $user -g $user -m 700 $memory

# 4. greetd starts the compositor of the login screen with the screen in it (the old file kept):
sudo cp -n /etc/greetd/config.toml /etc/greetd/config.toml.bak-sherlockos
printf '%s\n' '[terminal]' 'vt = 7' '' '[default_session]' 'command = "$command"' 'user = "$user"' | sudo tee /etc/greetd/config.toml

# 5. greetd instead of SDDM, from the next boot:
sudo systemctl disable sddm.service
sudo systemctl enable greetd.service
sudo reboot

# To go back to SDDM (from a text console, Ctrl + Alt + F3, if the login screen does not show):
#   sudo systemctl disable greetd.service
#   sudo systemctl enable sddm.service
#   sudo reboot
# To remove the login screen afterwards:
#   sudo rm -r $dest/sherlockos-greeter $memory
#   sudo mv /etc/greetd/config.toml.bak-sherlockos /etc/greetd/config.toml
EOF
