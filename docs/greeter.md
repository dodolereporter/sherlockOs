# Login screen

`sherlockos-greeter` (crate `desktop/greeter`, lot D-24) is the login screen of SherlockOS. It
replaces SDDM and runs under greetd (ADR 0026): greetd runs PAM and starts the session; this
program asks who logs in and relays PAM's conversation. greetd starts it, as its `_greetd`
user, inside our compositor in its login screen mode:

```toml
# /etc/greetd/config.toml
[default_session]
command = "/usr/local/libexec/sherlockos/compositor --greeter -- /usr/local/libexec/sherlockos/sherlockos-greeter"
user = "_greetd"
```

The compositor's side is "Login screen" in `docs/compositor.md`: one program fullscreen, no
shortcut of the desktop, the volume and brightness keys kept, and it stops with the greeter.

## What it shows (`src/app.rs`)

macOS-like, as the author asked, with the look of the lock screen (`docs/lock.md`): no board
draws it, the board of the login screen inspires it. The sizes and texts that are its own are in
`sherlock_ui::greeter`; the rest comes from `sherlock_ui::lock::screen`, and the background,
Sherlock's moods, the machine buttons and the pieces of the view from the `lock` crate.

- **Every screen** has an overlay surface of its own (layer-shell, one per screen, the keyboard
  taken): the picture of the lock, the date and the large time.
- **The screen of the pointer** (the first one at start) also shows, from the top:
  - Sherlock in 3D in his bubble, **the only avatar** (lot D-24d, as on macOS), which follows
    the theme as on the lock screen (lot D-25d; his still picture in software or without the
    model). The bubble and the name are the lock screen's own pieces (`lock::view::bubble`,
    `lock::view::name`);
  - the name of the selected user, or a pill to type one for « Autre… »;
  - **when there are several users**, a discreet row of small pills under the name, one per
    user (their name, « … » when too long) and « Autre… » last; the selected one is of glass,
    the others show their glass on hover. A click or the left and right arrows (while nothing
    is typed) choose another one: the new name fades in and Sherlock looks at the pills for
    1.2 s. A single user: only their name, and no « Autre… »;
  - the password pill with the eye and the arrow, and the line under it;
  - **the session**: a discreet segmented control, « Bureau » (SherlockOS) and « Mode jeu »,
    grayed until the game mode exists; under it « Autres sessions » opens the list of the
    others (Plasma), and once one is chosen the line says « Session : Plasma (Wayland) »;
  - the machine buttons, « Veille », « Redémarrer », « Éteindre », each confirmed by a second
    click (`lock::machine`), run by `systemctl`: logind decides whether greetd's user may.
    « Redémarrer » and « Éteindre » leave as on the lock screen: Sherlock waves, the screen
    fades to black, then the command (lot D-25i);
  - in the bottom right corner, the network, the keyboard layout and the battery, discreet
    (`lock::status`, read on a thread, bounded; `greeter-status-1366x768`).
- **Sizes follow the screen** (`lock::screen::fit`): tried on 1366 × 768 and 1920 × 1080.

## Behavior

- **Background (lot D-24d):** the surfaces are transparent. The compositor shows the picture of
  the lock from its first frame (`docs/compositor.md`), and this program's own picture, once
  read, fades in over it: the same picture, so nothing moves. It never draws a plain color.
- **The keyboard stays in the field** (lot D-24d, as on macOS): the password field, or the name
  field for « Autre… », takes it from the first frame without a click, and gets it back after a
  wrong password, a change of user, a click on the eye, a session, a machine button or anywhere
  on the screen. The keys typed during the entrance go into it. Escape empties it. Tab goes from
  the field to the eye, the arrow and the machine buttons, and back (`sherlock_ui::focus`).
- **Entrance:** from the first frame drawn, the time, the bubble, the name, the pill, the session
  and the buttons rise and fade in, 70 ms apart; the first key ends it.
- **Sherlock** reacts as on the lock screen: he waves, closes his eyes while the password is
  typed, peeks with one eye when it is shown, thinks while greetd checks, is sorry on a
  refusal, happy once logged in. While keys come in a field, and 1.5 s after the last one, he
  looks down at it, his head bent and his open eyes turned (`lock::form::gaze`).
- **Enter:** the field empties at once, the pill shrinks into a circle with the spinner while
  greetd and PAM check, on a thread of their own (`login::log_in`).
- **Refused:** the pill comes back and shakes three times, « Mot de passe incorrect », the user
  stays selected. greetd unreachable or failing: « Connexion impossible. Réessaie. », or PAM's
  own message when it gave one.
- **Logged in:** the user and their session are remembered (`memory`), the session is started
  through greetd, Sherlock is happy for 700 ms, then the elements leave in reverse order (the
  buttons first, the time last, 70 ms apart, `lock::screen::exit`), the bubble shrinking, and
  the program ends; the background stays to the end. The compositor stops with it and greetd
  starts the session, whose compositor starts from the same picture. In the session, the compositor plays
  the arrival of the desktop: the bar slides down from the top, then the dock rises from the
  bottom, on a spring, and the windows open with their own animation (see "Arrival of the
  desktop" in `docs/compositor.md`).
- **The last user** is selected at start, with the session they chose last time (SherlockOS by
  default).

## Users, sessions, memory

- `src/users.rs`: the human users of `/etc/passwd` (ids from `UID_MIN` to `UID_MAX` of
  `/etc/login.defs`, 1000 to 60000 by default; a shell listed in `/etc/shells`, not `nologin`
  nor `false`), real names from the comment field. Files read with a bound of 4 MiB. The
  screen shows that list first; AccountsService is asked afterwards on a thread of its own
  (lot D-24f), each call within 2 s, 64 accounts at most: when it answers, its system accounts
  are left out and its real names taken, the selection staying on the same user. A slow or
  stuck AccountsService never holds the first frame.
- `src/sessions.rs`: the entries of `wayland-sessions` in the data folders (`/usr/local/share`,
  then `/usr/share`): hidden ones, those without `Exec` or whose `TryExec` is missing left out,
  the first of an id kept, SherlockOS first. `Exec` is split as the desktop entry specification
  quotes it; greetd gets it with `XDG_SESSION_TYPE=wayland`, `XDG_SESSION_DESKTOP` and
  `XDG_CURRENT_DESKTOP` from `DesktopNames`. greetd runs the command through
  `/bin/sh -c "exec " + cmd.join(" ")` (`greetd/src/session/worker.rs`): each word is sent
  quoted for `sh` (`greetd::shell_word`, lot D-24f), so that a path with a space, a quote or
  `$ ; & *` reaches the session whole and is never read by the shell.
- `src/memory.rs`: `/var/cache/sherlockos-greeter/state` (or `SHERLOCKOS_GREETER_STATE`), owned
  by `_greetd`: `last-user=` and one `session.<user>=` line per user (32 at most). Read with a
  bound, written whole then renamed; a name with a line break or `=` is never written.

## Installing it

`just install-greeter` builds the compositor and the screen in release, then prints the
commands (`desktop/greeter/install.sh`); it runs none of them. The programs come from the
release folder of `CARGO_TARGET_DIR` (relative to the repository), or of `target/`. Each must
be up to date (`desktop/compositor/session/built.sh`, shared with `just install-session`): no
source that cargo lists in its dep-info file (`<program>.d`) newer than it, and built from this
repository, not from another checkout sharing the target folder. Otherwise the script prints
nothing and says which program to build again (lot D-24f). The session itself comes from
`just install-session`, whose commands go first. In order:

1. `sudo apt install greetd`: Debian's package (0.10.3 in trixie) creates the `_greetd` user,
   and enables nothing while SDDM holds `display-manager.service`;
2. the compositor and `sherlockos-greeter` in `/usr/local/libexec/sherlockos/`, the pictures of
   the lock in `/usr/local/share/sherlockos/lock/`, and the model of Sherlock (from `just
   mascot`, if it was downloaded) in `/usr/local/share/sherlockos/mascot/`: places `_greetd`
   can read, found through its default `XDG_DATA_DIRS`;
3. `/var/cache/sherlockos-greeter`, owned by `_greetd`, mode 700, for the memory;
4. `/etc/greetd/config.toml` (the old one kept as `config.toml.bak-sherlockos`): VT 7, the
   command `compositor --greeter -- sherlockos-greeter`, the user `_greetd`;
5. `systemctl disable sddm.service`, `systemctl enable greetd.service`, then a reboot.

**Back to SDDM**, printed with the rest, from a text console (Ctrl + Alt + F3) if the screen
does not show: `systemctl disable greetd.service`, `systemctl enable sddm.service`, reboot.

`tests/install.rs` runs the script on a fake repository whose path holds a space, under
`env -i`: the commands to switch and to go back are printed, the path quoted, the model
installed only when it was downloaded; without the build, nothing is printed. The build of
`CARGO_TARGET_DIR` is the one installed; a program older than one of its sources, or built from
another checkout, is refused.

## Switching user without closing the session (study, not done)

The lock screen should offer « Changer d'utilisateur »: open a second session while the first
keeps running. greetd does not do it by itself:

- one greetd serves one console, and runs one session at a time: it starts its greeter again
  only once the session ended (`greetd(1)`); a second `create_session` while a session runs is
  refused (« a session is already being configured », « session already active »);
- greetd can run several times, one instance per console (`greetd --vt 8 --config …`), each
  with its own socket (`/run/greetd-<pid>.sock`) and its own greeter.

So « Changer d'utilisateur » could start a second greetd on a free console (a templated unit,
`greetd@tty8.service`, started through a polkit rule for the active session), switch to it, and
let the other user log in there; logind keeps both sessions, and Ctrl + Alt + Fn, or our own
switcher, goes back. What it costs: a unit and a polkit rule of ours, a choice of consoles, and
a way back to the first session from the second one's screen (logind's `ActivateSession`). It
needs the author's go-ahead (an ADR), and a test on the machine (rule 9).

## greetd's IPC (`src/greetd.rs`)

greetd gives its greeter the path of a socket in `GREETD_SOCK` (`greetd-ipc(7)`). Each message is
a JSON object behind its length, 32 bits in the machine's byte order. The requests are
`create_session` (the user name), `post_auth_message_response` (the answer to PAM's last
message, none for an information), `start_session` (the command and the environment of the
session) and `cancel_session`; the replies `success`, `error` (`auth_error` for a refused
authentication) and `auth_message` (`visible`, `secret`, `info` or `error`).

- **No library for it.** The client is ours, about 150 lines: writing the requests by hand lets
  the password go into one buffer of its exact final size, wiped with `zeroize` once sent, with
  no growing buffer that would leave a copy behind. serde_json reads the replies.
- **Bounded.** A reply longer than 64 KiB is refused as a broken socket.
- **Never left half done.** A `Greetd` dropped with a session being created (the screen stops,
  or panics and unwinds) sends `cancel_session` first; greetd also ends a session whose greeter
  is gone.
- **Nothing logged.** `Debug` of the password prints `***`; no request is ever logged.

## A login (`src/login.rs`)

`log_in(greetd, user, password, launch)` creates the session, answers PAM, then starts the
session:

- the first secret prompt gets the password, which is then wiped;
- information and error messages are kept for the screen (`Report::notes`) and acknowledged;
- `auth_error` is a refusal (`Outcome::Refused`): a wrong password or an unknown user, which
  PAM does not tell apart; the session is cancelled, as greetd's own greeters do, so that the
  next attempt starts afresh on the same connection;
- any other error is reported with greetd's reason (`Outcome::Failed`), session cancelled;
- a second secret prompt (a password to change, a second factor) or a visible one is not
  handled yet: the session is cancelled and PAM's question is reported
  (`Outcome::Unsupported`), for the screen to show;
- once authenticated, `start_session` with the command and environment of the chosen session;
  greetd starts it once the greeter has left.

## Tests

`tests/fake_greetd.rs` plays greetd on a socket of the test's own (no PAM, no greetd, no session
of the machine): the right password starts the session, with its command and environment; a
wrong one is refused, the session cancelled, and the next attempt on the same connection goes
through; an information message is kept and acknowledged without a response; an unknown user is
refused like a wrong password, or reported when greetd refuses it at once, the password never
sent; a session that cannot start is reported and cancelled; a second secret prompt is given up
with its question, the password sent once; a greeter that leaves mid-login cancels its session;
without greetd, the connection fails; a command whose path holds a space is sent quoted for
`sh`; a greetd that takes the password and never answers is given up after the delay (60 s,
`REPLY_TIMEOUT`; 200 ms in the test), the session cancelled and the connection closed, the next
attempt opening a new one. `src/greetd.rs` checks the JSON of each request, a password
with quotes, backslashes, control characters and non-ASCII letters escaped into a frame of its
exact size, the replies read or refused, and the words of a command given to a real `sh -c
"exec …"` coming out whole (spaces, quotes, `$`, `;`, `*`, backquotes, an empty word).

`src/users.rs`, `src/sessions.rs` and `src/memory.rs` test the list of users from a fake
`passwd` (range, shells, `login.defs`, AccountsService's names and system accounts), the
sessions from fake folders and the splitting of `Exec`, and the memory read back, its
forbidden names and the file replaced whole.

`tests/render.rs` drives the screen with a fake greetd and a fake memory: the last user is
selected with their session, the arrows go through the users and stay in the field once
something is typed; the right password goes through Checking to Leaving, Sherlock happy, the
user and session remembered; a wrong one shakes and keeps the user; greetd unreachable says so;
« Autre… » asks for a name; a single user is only a name; Sherlock looks at the field while
keys come; AccountsService answering after the first frame keeps the selection on the same
user. It writes the screenshots to `target/tmp/greeter/`: two in the CI (at rest,
dark, 1920 × 1080; the error, light, 1366 × 768), and with `-- --ignored` all of them: at rest,
a user being chosen and chosen, typing, the error, and « Autre… » with the other sessions, in
the dark and the light theme, at 1920 × 1080 and 1366 × 768. The boards of the entrance and of
the exit, 6 frames each (`board-entrance.png`, `board-exit.png`):
`cargo test -p greeter --test render -- --ignored boards`.

```sh
cargo test -p greeter
```
