# Sourced by the install scripts (install.sh here and in desktop/greeter): where cargo put the
# release build, and whether a program of it is up to date with the repository it installs from.

# release_dir REPO: the release folder of CARGO_TARGET_DIR, as `just` runs cargo from REPO (a
# relative CARGO_TARGET_DIR is relative to REPO), or REPO/target/release.
release_dir() {
	target="${CARGO_TARGET_DIR:-target}"
	case "$target" in
	/*) ;;
	*) target="$1/$target" ;;
	esac
	printf '%s/release\n' "$target"
}

# check_built REPO RELEASE PROGRAM...: fails with a message on the first program of RELEASE that
# is missing, that was not built from the sources of REPO, or that is older than one of its
# sources. The sources are those cargo lists in the dep-info file beside the program
# (PROGRAM.d): every file of every crate it was built from.
check_built() {
	repo=$1
	real=$(cd "$repo" && pwd -P)
	release=$2
	shift 2
	# Cargo writes a space inside a path as "\ ": kept as this character while splitting.
	soh=$(printf '\001')
	for program in "$@"; do
		binary="$release/$program"
		again="cargo build --release --bin $program"
		if [ ! -x "$binary" ]; then
			echo "missing $binary: $again" >&2
			return 1
		fi
		if [ ! -f "$binary.d" ]; then
			echo "$binary has no dep-info file ($binary.d) to check it against its sources: $again" >&2
			return 1
		fi
		verdict=$(head -n 1 "$binary.d" | sed -e "s/\\\\ /$soh/g" -e 's/^[^ ]*: *//' |
			tr ' ' '\n' | tr "$soh" ' ' | {
			ours=""
			while IFS= read -r source; do
				case "$source" in
				"") continue ;;
				"$repo"/* | "$real"/*) ours=1 ;;
				esac
				if [ "$source" -nt "$binary" ]; then
					printf 'older than %s\n' "$source"
					exit
				fi
			done
			[ -n "$ours" ] || echo "not built from $repo"
		})
		if [ -n "$verdict" ]; then
			echo "$binary is $verdict: $again" >&2
			return 1
		fi
	done
}
