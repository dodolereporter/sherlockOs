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

```sh
cargo test -p greeter
```
