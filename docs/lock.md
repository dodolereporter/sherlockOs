# Lock screen

`sherlockos-lock` (crate `desktop/lock`, lot D-25a2) shows the lock screen and checks the
password. The compositor holds the lock itself (`ext-session-lock-v1`, see "Session lock" in
`docs/compositor.md`), and the security model is ADR 0023: this program only draws the screen
and asks PAM. If it crashes, the session stays locked and the compositor starts it again.

## What it shows

macOS-like, as the author asked; no board draws it (the board of the login screen inspires it). Every
color, size, duration and text is in `sherlock_ui::lock::screen`.

- **Every screen** has a lock surface of its own, at its size and scale, on the background of
  the system screens, with the date (« jeudi 26 novembre ») and the large time at the top (regular Inter: iced 0.14
  draws the variable font at its default weight only, a thin weight would fall back to another
  font).
- **The screen of the pointer** also shows, at the bottom middle: the round bubble of Sherlock
  (the still picture `docs/design/images/sherlock-neutre.png`; the 3D model comes with D-25b),
  the user's name, the password pill of glass with the eye (show or hide the password) and the
  arrow, the line of the error under it, then the machine buttons: « Veille », « Redémarrer »,
  « Éteindre ». The pill and the buttons move to the screen the pointer enters, or the one that
  gets the keyboard; the compositor gives the keyboard to the lock surface of the screen under
  the pointer.
- **Sizes follow the screen.** The time, the room above it, the bubble and the gaps are a share
  of the screen's logical height, kept between a minimum and a maximum (`screen::fit`); the
  rest has fixed logical sizes, which the screen's scale multiplies. Tried on 1366 × 768,
  1920 × 1080, 2560 × 1600 at 1.5 and 3840 × 2160 at 2.
- **Background:** plain, for now. The blurred and darkened wallpaper of the board is left for
  later: the compositor has no blur yet (lot D-37), and reading and blurring the wallpaper in the
  client would slow the first frame of the lock.

## Behavior

- **Entrance:** the time, the bubble, the name, the pill and the buttons rise into place and fade
  in, 70 ms apart, on the spring of a window that opens. The first key ends every animation at
  once: typing is never slowed down.
- **Enter or the arrow:** the field empties at once, the pill shrinks into a circle and the arrow
  becomes a spinner while PAM checks the password, on a thread of its own.
- **Refused:** the pill comes back, shakes three times and stays empty, with « Mot de passe
  incorrect » under it (or « Trop d'essais… » when PAM says so, or « Vérification
  impossible… »); the keyboard stays in the field.
- **Accepted:** everything fades out in 220 ms, then the program ends the lock
  (`unlock_and_destroy`) and leaves.
- **Machine buttons:** a first click arms the button (amber, « Reclique pour confirmer »), a
  second click at least 400 ms later runs `systemctl suspend`, `reboot` or `poweroff`; the button
  disarms after 4 seconds or when the password is typed. logind decides whether it is allowed.

## Password and PAM

- PAM only, never a check of our own, nothing stored (ADR 0023). The service is
  `sherlockos-lock` (`desktop/lock/pam/sherlockos-lock`: `auth` and `account` of `login`, like
  swaylock), for the user of the session only, read from the password database (`getpwuid_r`),
  not from `$USER`. Without `/etc/pam.d/sherlockos-lock`, PAM uses its `other` service, which
  refuses every password on Debian: `sherlockos-session` writes a warning in the session log,
  and `just install-session` prints the `sudo install` line, which the author runs.
- `libpam.so.0` is opened at run time (`libloading`): nothing to build against, no development
  package.
- The password is a `Secret` (`system_services::secret`, shared with the Wi-Fi password of the
  bar): `Debug` prints `***`, and it is overwritten with zeros when dropped. The conversation
  function gives PAM a `malloc` copy, which PAM frees; our copy is overwritten as soon as it is
  handed over. It is never logged.
- pam_faillock, when the system's `login` uses it, limits the tries; its delay is respected
  (the check runs on its own thread, the screen keeps drawing).

## Tests

- `src/pam.rs`: the conversation, driven like PAM drives it with fake messages (the hidden
  prompt gets the password once, the visible one the user, the others nothing); never the
  system's PAM, never a real password.
- `src/machine.rs`: the second click and its delay.
- `tests/render.rs`, with a fake PAM and a fake `systemctl`: the right password unlocks, a wrong
  one empties the field, an empty one sends nothing; the typed password never shows in a
  message (`Typed(***)`); the pill follows the screen of the pointer; a machine button needs a
  second click. Screenshots in `target/tmp/lock/`: rest, typing, spinner and error in both
  themes, the second screen, and the rest on the four sizes above.

## Manual test (rule 9)

On the reference laptop, the PAM service installed:

1. Super + L: the lock screen comes, nothing of the desktop shows. Type a wrong password: the
   pill shakes and empties. The right one: the desktop comes back.
2. Lock, then from another console `pkill -KILL sherlockos-lock`: back on the session's
   console, the session is still locked, and the lock screen comes back.
3. Plug a screen in while locked: it shows the background and the time; the pill follows the
   pointer.
4. « Veille »: two clicks, the laptop sleeps; at wake-up it is still locked.
