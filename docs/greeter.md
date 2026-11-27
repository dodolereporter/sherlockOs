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
  - Sherlock in 3D in his bubble, which follows the theme as on the lock screen (lot D-25d;
    his still picture in software or without the model);
  - **the row of users**: a glass bubble with the initial of each human user, the selected one
    larger with a ring in the color of the text, in the middle of the screen; « Autre… » (a
    person) last. The others are dimmed. A click or the left and right arrows (while nothing
    is typed) choose another one: the row slides on a spring and Sherlock looks at the new user for 1.2 s. For
    now only his eyes turn: `sherlock-3d` aims the eye bones only, and the turn of the head
    waits for it;
  - the name of the selected user, or a pill to type one for « Autre… »;
  - the password pill with the eye and the arrow, and the line under it;
  - **the session**: a discreet segmented control, « Bureau » (SherlockOS) and « Mode jeu »,
    grayed until the game mode exists; under it « Autres sessions » opens the list of the
    others (Plasma), and once one is chosen the line says « Session : Plasma (Wayland) »;
  - the machine buttons, « Veille », « Redémarrer », « Éteindre », each confirmed by a second
    click (`lock::machine`), run by `systemctl`: logind decides whether greetd's user may.
- **Sizes follow the screen** (`lock::screen::fit`): tried on 1366 × 768 and 1920 × 1080.

## Behavior

- **Entrance:** the time, the bubble, the row, the pill, the session and the buttons rise and
  fade in, 70 ms apart; the first key ends it.
- **Sherlock** reacts as on the lock screen: he waves, closes his eyes while the password is
  typed, peeks with one eye when it is shown, thinks while greetd checks, is sorry on a
  refusal, happy once logged in.
- **Enter:** the field empties at once, the pill shrinks into a circle with the spinner while
  greetd and PAM check, on a thread of their own (`login::log_in`).
- **Refused:** the pill comes back and shakes three times, « Mot de passe incorrect », the user
  stays selected. greetd unreachable or failing: « Connexion impossible. Réessaie. », or PAM's
  own message when it gave one.
- **Logged in:** the user and their session are remembered (`memory`), the session is started
  through greetd, Sherlock is happy for 700 ms, then the elements leave in reverse order, the
  bubble shrinks, the background zooms in to 1.05 and fades, and the program ends; the
  compositor stops with it and greetd starts the session.
- **The last user** is selected at start, with the session they chose last time (SherlockOS by
  default).

## Users, sessions, memory

- `src/users.rs`: the human users of `/etc/passwd` (ids from `UID_MIN` to `UID_MAX` of
  `/etc/login.defs`, 1000 to 60000 by default; a shell listed in `/etc/shells`, not `nologin`
  nor `false`), real names from the comment field. When AccountsService answers on the system
  bus, its system accounts are left out and its real names taken. Files read with a bound of
  4 MiB.
- `src/sessions.rs`: the entries of `wayland-sessions` in the data folders (`/usr/local/share`,
  then `/usr/share`): hidden ones, those without `Exec` or whose `TryExec` is missing left out,
  the first of an id kept, SherlockOS first. `Exec` is split as the desktop entry specification
  quotes it; greetd gets it with `XDG_SESSION_TYPE=wayland`, `XDG_SESSION_DESKTOP` and
  `XDG_CURRENT_DESKTOP` from `DesktopNames`.
- `src/memory.rs`: `/var/cache/sherlockos-greeter/state` (or `SHERLOCKOS_GREETER_STATE`), owned
  by `_greetd`: `last-user=` and one `session.<user>=` line per user (32 at most). Read with a
  bound, written whole then renamed; a name with a line break or `=` is never written.

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
without greetd, the connection fails. `src/greetd.rs` checks the JSON of each request, a password
with quotes, backslashes, control characters and non-ASCII letters escaped into a frame of its
exact size, and the replies read or refused.

`src/users.rs`, `src/sessions.rs` and `src/memory.rs` test the list of users from a fake
`passwd` (range, shells, `login.defs`, AccountsService's names and system accounts), the
sessions from fake folders and the splitting of `Exec`, and the memory read back, its
forbidden names and the file replaced whole.

`tests/render.rs` drives the screen with a fake greetd and a fake memory: the last user is
selected with their session, the arrows go through the users and stay in the field once
something is typed; the right password goes through Checking to Leaving, Sherlock happy, the
user and session remembered; a wrong one shakes and keeps the user; greetd unreachable says so;
« Autre… » asks for a name. It writes the screenshots to `target/tmp/greeter/`: two in the CI (at rest,
dark, 1920 × 1080; the error, light, 1366 × 768), and with `-- --ignored` all of them: at rest,
a user being chosen and chosen, typing, the error, and « Autre… » with the other sessions, in
the dark and the light theme, at 1920 × 1080 and 1366 × 768.

```sh
cargo test -p greeter
```
