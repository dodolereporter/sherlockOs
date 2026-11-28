# ADR 0026: The login screen runs on greetd, in our compositor's `--greeter` mode

- Status: proposed
- Date: 2026-11-27
- Lot: D-24

## Context

SherlockOS needs its own login screen, with the look of the lock screen (Sherlock in 3D, the
pill, the machine buttons) and a choice of user and session. On the author's machine, SDDM
starts Plasma and, since D-14, the SherlockOS session (`sherlockos.desktop`). SDDM draws its
screen with QML themes: Sherlock in 3D, our design system and our animations do not fit in one.

`docs/contexte.md` already says it: "the login screen relies on greetd for the
authentication; we only write the interface". greetd (Debian package `greetd`, 0.10.3 in
trixie) is a small daemon: it runs PAM (its `greetd` service) and starts the session, and lets a
program of our own, the greeter, ask who logs in, over a socket (`greetd-ipc(7)`). The greeter
is any program greetd starts as an unprivileged user (`_greetd` on Debian). A graphical greeter
needs a Wayland compositor to show in.

## Decision

- **greetd replaces SDDM.** Its `default_session` is
  `compositor --greeter -- sherlockos-greeter`, run as the `_greetd` user that the Debian
  package creates (greetd's own default name is `greeter`; the name is the only difference).
  `just install-greeter` prints the commands that install it and those that go back to SDDM; it
  never runs them.
- **Our compositor gains a `--greeter` mode** rather than a second compositor (cage, sway):
  one program shown fullscreen, no shortcut of the desktop (Mission Control, launcher,
  workspaces, snapping, screenshots, gestures), no shell socket, nothing that locks; the console
  switch, Ctrl + Alt + Backspace and the volume and brightness keys stay. The compositor stops
  with the greeter, so that greetd starts the session.
- **The greeter is `sherlockos-greeter`** (crate `desktop/greeter`), an iced program of the
  shell. It speaks greetd's IPC through a small client of our own (`greetd.rs`) rather than the
  `greetd_ipc` crate: the protocol is four requests and three replies of JSON, and writing the
  requests by hand lets the password go into one buffer of its exact size, wiped once sent.
  serde_json, already in the workspace, reads the replies.
- **The session it starts** is read from `/usr/share/wayland-sessions`: `sherlockos.desktop`
  ("Bureau"), and the others (Plasma) in a discreet list, as a way back.

## Consequences

- The login screen, the lock screen and the desktop share one compositor, one toolkit and one
  design system: the same Sherlock, the same pill, and the hand-over to the session can be
  animated by our own code.
- PAM stays in greetd: the greeter never checks a password itself, it relays PAM's
  conversation (prompts, messages, errors). A second secret prompt (a password to change) is
  not handled at first: the screen shows PAM's question and gives up.
- The greeter runs as `_greetd`: what it reads (the model of Sherlock, the backgrounds) must be
  readable by that user, so it is installed under `/usr/local/share/sherlockos/`. What it
  remembers (the last user, each user's session) goes to `/var/cache/sherlockos-greeter/`,
  owned by `_greetd`.
- The compositor of the login screen runs the GPU code of a whole session as an unprivileged
  user before anyone logs in; a crash there brings greetd back to its greeter, never into a
  session.
- greetd runs one session at a time per console: switching user without closing the session
  (asked for the lock screen) needs a second greetd, on another console (see the report of
  D-24).
- Going back to SDDM is two `systemctl` commands, printed by `just install-greeter`.

## What would make us reconsider

- greetd stops being maintained, or leaves Debian.
- The `--greeter` mode grows into a second compositor inside the first: then a separate small
  compositor for the login screen would be simpler.
- Fast user switching turns out to need a display manager that handles several sessions
  itself (GDM, SDDM do), and the author wants it more than our own screen.
