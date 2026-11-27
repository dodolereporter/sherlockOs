# Lock screen

`sherlockos-lock` (crate `desktop/lock`, lots D-25a2 and D-25b) shows the lock screen and checks the
password. The compositor holds the lock itself (`ext-session-lock-v1`, see "Session lock" in
`docs/compositor.md`), and the security model is ADR 0023: this program only draws the screen
and asks PAM. If it crashes, the session stays locked and the compositor starts it again.

## What it shows

macOS-like, as the author asked; no board draws it (the board of the login screen inspires it).
Every color, size, duration and text is in `sherlock_ui::lock::screen`.

- **At rest** (lot D-25b), every screen has a lock surface of its own, at its size and scale,
  with only the background, the date (« jeudi 26 novembre ») and the large time at the top
  (regular Inter: iced 0.14 draws the variable font at its default weight only, a thin weight
  would fall back to another font). No form, as on macOS.
- **A key, a click or the pointer** bring the form on the screen of the pointer, at the bottom
  middle: the round bubble of Sherlock in 3D (`sherlock_3d::Sherlock3d`; the still picture
  `docs/design/images/sherlock-neutre.png` in software or without the model), the user's name, the password pill of glass with the eye (show or hide the password) and the
  arrow, the line of the error under it, then the machine buttons: « Veille », « Redémarrer »,
  « Éteindre ». The pill and the buttons move to the screen the pointer enters, or the one that
  gets the keyboard; the compositor gives the keyboard to the lock surface of the screen under
  the pointer.
- **Sizes follow the screen.** The time, the room above it, the bubble and the gaps are a share
  of the screen's logical height, kept between a minimum and a maximum (`screen::fit`); the
  rest has fixed logical sizes, which the screen's scale multiplies. Tried on 1366 × 768,
  1920 × 1080, 2560 × 1600 at 1.5 and 3840 × 2160 at 2.
- **Background** (lot D-25b), in this order: `background = /absolute/path` in
  `~/.config/sherlockos/lock.conf` (`desktop_config::lock`), shown as is; else the pictures of
  the lock given by the author, `assets/lock/lock-dark.jpg` and `lock-light.jpg` (a desk seen
  from above, the middle left free for the time and the bubble), which `just install-session`
  puts in `/usr/local/share/sherlockos/lock/` and which are looked for in the data folders of
  the desktop, one per theme; else the wallpaper of the desktop, blurred and darkened. Each
  picture is decoded once, on a thread, once the size of the screens is known, and reduced to
  cover the largest one in pixels (logical size times its scale): centered, neither stretched
  nor banded, never kept at 4K for a small screen. It follows the theme of the appearance
  file, with its fade (the light picture fades in over the dark one). The paths come from the
  user: a picture is read only from a regular file (links followed) of 64 MiB at most, read with
  that bound, and decoded with sides of 16 384 pixels and 256 MiB at most, reduced before any
  other copy; anything else (`/dev/zero`, a FIFO, a huge or lying file) falls to the next
  source or the plain background. The model of Sherlock follows the same rule in `sherlock-3d`
  (32 MiB at most, textures bounded too). The middle of the pictures
  is plain enough: no veil is laid behind the time or the field. The plain background of the
  system screens shows until the picture is ready.

## Behavior

- **Entrance:** the time comes with the screen. The form comes on demand (`src/form.rs`): the
  bubble grows from 60 % of its size on a spring, then the name, the pill and the buttons rise
  and fade in, 70 ms apart. A key once the form shows ends the animations: typing is never
  slowed down.
- **The key that brings the form is not lost:** a letter, a digit or a sign goes into the field;
  Space and Enter only bring the form (Enter sends nothing to PAM). Escape sends the form away.
- **Leaving:** after 20 s without input (unless PAM is checking), the form leaves in reverse
  order, and the field empties, the eye closed.
- **Sleep:** confirming « Veille » sends the form away and wipes the field first. The 20 s are
  counted on the monotonic clock, which stops during a suspension: so a jump of more than 5 s
  of the time since boot (`CLOCK_BOOTTIME`) between two ticks of a second (the lid closed, a
  sleep started elsewhere) sends the form away and wipes the field too. Nothing typed waits for
  whoever opens the lid.
- **Sherlock** follows the field: he waves (`Wave`) when the form comes, closes his eyes
  (`EyesClosed`) while something is typed, opens one eye (`Peek`) when the eye shows the
  password, opens them and looks at the field when it is empty again, thinks (`Think`) while
  PAM checks, is sorry (`Sorry`) for 2.5 s after a refusal, happy (`Success`) when it is
  accepted, and sleeps (`Sleep`) once « Veille » is confirmed. The model is read on a thread at
  start (`sherlock_3d::locate`); hidden, he costs nothing.
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
  bar): `Debug` prints `***`, and it is overwritten with zeros (`zeroize`, volatile writes)
  when dropped. The conversation function gives PAM a `malloc` copy, which PAM frees; our copy
  is overwritten as soon as it is handed over. It is never logged.
- The first key, which brings the form, travels as a `Secret` too (`form::Input`).
- **Limit: the copies of iced's text field are not wiped.** The `text_input` keeps the typed
  text in its own state, and `on_input` hands a new `String` at each key before it becomes a
  `Secret`; iced frees them without overwriting them. They live in the memory of this process
  only, for the time the field shows them; a process able to read it (ptrace) could read the
  password anyway.
- **Tries.** The lock screen runs as the user, so it cannot write the counters of
  pam_faillock: failures are not counted, as with swaylock. Each wrong password waits for the
  delay of pam_faildelay from `login` (3 s on Debian) before PAM answers, and only one check
  runs at a time: that is what slows the tries. The check runs on its own thread, the screen
  keeps drawing.
- **A PAM that hangs** (lot D-25f). The interface never waits for PAM: the check runs on a
  thread of its own, and after 20 s (`pam::TIMEOUT`) without an answer it counts as PAM
  unavailable: the spinner stops, the field comes back with « Vérification impossible. Réessaie. », and
  the next try starts a new check. A module waiting for a daemon (`pam_winbind`, in Debian's
  `common-auth` when Samba's winbind is installed) or for the network can no longer leave the
  lock screen in its spinner for good. The session log says « authentication: checking » at each
  try.
- **Only one clean exit.** The toolkit (`exwlshellev`) sends `unlock_and_destroy` on every
  clean exit while the lock is held: the program reaches `iced::exit()` only after PAM accepted
  the password. A panic aborts the process (no unwinding through the toolkit), and an error
  ends it without a clean exit: the compositor keeps the session locked. A test checks that
  the source has a single exit, in the branch of the acceptance.
- **Its connection.** The compositor starts the program with a connection it opened itself
  (`WAYLAND_SOCKET`), the only one allowed to lock (ADR 0023). The program opens no other: the
  clipboard is off.

## Tests

- `src/pam.rs`: the conversation, driven like PAM drives it with fake messages (the hidden
  prompt gets the password once, the visible one the user, the others nothing); never the
  system's PAM, never a real password.
- `src/machine.rs`: the second click and its delay.
- `src/form.rs`: the form on demand (a letter goes into the field, Space and Enter do not,
  Escape and 20 s without input send it away, not while PAM checks) and Sherlock's mood.
- `src/background.rs`: the picture reduced, blurred and darkened; `lock.conf` before the
  wallpaper.
- `tests/render.rs`, with a fake PAM and a fake `systemctl`: the form, the letter that brings
  it, Space and Enter, the form leaving and the field emptied, the mood of Sherlock; the right
  password unlocks, a wrong one empties the field, an empty one sends nothing; only PAM's
  acceptance leads to the exit; the typed password never shows in a message (`Typed(***)`);
  the pill follows the screen of the pointer; a machine button needs a second click.
  Screenshots in `target/tmp/lock/`, on the blurred wallpaper: rest (no form), form, typing,
  password shown (`peek`), spinner and error in both themes, the second screen, and the rest
  and the form on the four sizes above. In software, Sherlock is his still picture: the 3D
  bubble is seen on the machine.

## Manual test (rule 9)

On the reference laptop, the PAM service installed:

1. Super + L: the lock screen comes, nothing of the desktop shows. Type a wrong password: the
   pill shakes and empties. The right one: the desktop comes back.
2. Lock, then from another console `pkill -KILL sherlockos-lock`: back on the session's
   console, the session is still locked, and the lock screen comes back.
3. Plug a screen in while locked: it shows the background and the time; the pill follows the
   pointer.
4. « Veille »: two clicks, the laptop sleeps; at wake-up it is still locked.
5. (D-25b) Super + L: only the blurred wallpaper, the date and the time. Type a letter: the form
   comes, Sherlock waves, the letter is in the field, his eyes close. Press the eye: one eye
   opens. Empty the field: he looks at it. Wait 20 s: the form leaves, the field is empty.
   Space or Enter bring it back with an empty field.
6. (D-25b) `background = /path/to/picture.jpg` in `~/.config/sherlockos/lock.conf`: that picture,
   blurred, at the next lock.

## Rescue mode (lot D-25c)

`sherlockos-lock --rescue` is what the compositor starts once the lock screen failed three
times in a row (before `swaylock`, which no recipe installs): the same program, on a connection
the compositor opens itself, so the same identity rule (ADR 0023). It draws in software
(`gpu::Reason::Rescue`), without Sherlock, background, animation nor machine buttons: the line
« Écran de verrouillage de secours », the user's name, the password pill and its error line, in
the middle of the plain background of the system screens. PAM is called exactly as in the
normal mode. The field always shows: Escape and the 20 s without input do not send it away, but
a wake-up (a jump of `CLOCK_BOOTTIME`) still empties it. If it dies, the compositor starts it
again, three times, then `swaylock` if installed, then its own screen.

Tests: `tests/render.rs` (the rescue unlocks with a fake PAM, keeps its field; capture
`rescue-tiny-skia.png`), and the compositor's `tests/session_lock.rs`: the lock program fails
three times, the rescue mode (a child process of the test, on the connection the compositor
gave it) takes the lock, an ordinary client can neither lock nor unlock meanwhile, and the
rescue killed is started again and takes the lock again.

