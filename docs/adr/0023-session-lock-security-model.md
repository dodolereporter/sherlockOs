# ADR 0023: Security model of the session lock

- Status: accepted (the author decided the two open questions on 2026-11-26, after the
  security review of lot D-25a)
- Date: 2026-11-26
- Lot: D-25a

## Context

Lot D-25 brings the session lock. A lock screen is only worth something if nothing gets
around it: no window may be seen or reached while the session is locked, and the session
must stay locked whatever happens to the program that draws the lock screen. The author
decided (brief of D-25, 2026-11-26) that the lock rests on the `ext-session-lock-v1` standard,
applied strictly by our compositor, and on PAM for the password: no password check of our
own, nothing stored.

`ext-session-lock-v1` splits the work: the compositor locks and hides, a client (the lock
screen) shows the screen and asks for the password, then ends the lock with
`unlock_and_destroy`. Smithay 0.7 implements the protocol (`wayland::session_lock`), but leaves
the policy to the compositor, and has two gaps for a strict use:

- it calls `SessionLockHandler::unlock` on `unlock_and_destroy` from **any** lock object, even
  one the compositor refused (`finished`), and even after posting the `invalid_unlock` error;
- `new_surface` does not say which lock a surface belongs to.

## Decision

**The compositor holds the lock; the client only draws it.**

1. **What is shown.** From the moment the session locks, each screen shows only its lock
   surface, in front of the plain background of the system screens (`sherlock_ui::lock`), or
   that background alone while it has none. No window, anchored surface (bar, dock,
   notifications, launcher), menu, Mission Control, closing animation or screenshot area is
   drawn: `render::items` returns the lock's items before building anything else. The
   compositor's own on-screen display of the volume and brightness keys stays in front.
   Screenshots go through the same items, so a capture can only show the lock.
2. **`locked` is sent after the frame.** The lock is confirmed to the client only once every
   screen has shown a frame built while locked (at the vertical blank in the real session).
   While the console is away, nothing is drawn, so the confirmation waits for it to come back.
3. **What reaches the input.** The keyboard focus can only be a lock surface (that of the
   screen under the pointer); `set_keyboard_focus` is the only place that sets it, and it
   ignores any other target while locked. The pointer only reaches the lock surface under it;
   pointer grabs, menus, gestures in progress, Mission Control and the screenshot area are
   cancelled at the lock. Touchpad gestures are dropped. The compositor's shortcuts are off,
   except:
   - Ctrl + Alt + F1 to F12, the console switch: the session stays locked on its console;
   - the volume and brightness keys (not the players);
   - Enter on the rescue screen (below).
   Power off and sleep through the power button belong to logind, not to the compositor.
   Ctrl + Alt + Backspace, which stops the compositor, does nothing while locked (amendment
   of lot D-25j, below): it goes to the lock surface as a Backspace.
4. **Only the lock that holds the session ends it.** The requests of `ext_session_lock_v1` go
   through our own dispatch before smithay's: `unlock_and_destroy` is refused with
   `invalid_unlock` (a protocol error, which disconnects the client) unless it comes from the
   lock the compositor accepted and confirmed. A surface created on a lock that was refused is
   never shown nor given input.
5. **Who may lock: only our lock program** (the author's decision of 2026-11-26). The
   compositor starts the lock program itself and opens its Wayland connection itself: a socket
   pair, one end plugged into the display (`insert_client`), the other handed to the program
   through `WAYLAND_SOCKET`, inherited by it alone. It recognizes the program by that
   connection (its `ClientId`), never by a pid, which a process of the session could take over
   once the program is dead. Only the connection of the last program started, alive, may lock,
   the session locked or not; any other client's `lock` is refused (`finished`), so a program
   of the session cannot show a lock screen of its own either. When the program dies or ends,
   its connection is forgotten and closed. `swaylock`, the rescue, is started the same way. A
   second lock while one is held is refused.
6. **The client dies, the session stays locked.** If the lock object goes away without
   `unlock_and_destroy` (crash, kill, disconnection), the session stays locked on the plain
   background, and the compositor starts the lock program again (`sherlockos-lock`), after a
   pause of 500 ms. A lock given up before `locked` takes its surfaces with it: they are no
   longer drawn nor given the keyboard. A program that ends without locking, or does not lock within 10 seconds,
   counts as a failure. After 3 failures in a row, it starts the rescue mode of the same program
   (`sherlockos-lock --rescue`, lot D-25c: a plain field, PAM as in the normal mode, in
   software), 3 times, through the same connection the compositor opens; then `swaylock` if it
   is installed; only then it draws a screen itself, which cannot unlock: « Verrouillé », and
   Enter to try the programs again. The counter starts again at each unlock.
7. **The password** is checked by PAM only, in the lock program, for the user of the session
   only (lot D-25a2). The compositor never sees it.

## Consequences

- A crash of the lock screen never shows the desktop: at worst the user sees the rescue
  screen, and gets out of it with Enter, or another console.
- If the compositor stops or crashes while locked, the logind session ends (amendment of lot
  D-25j, below): nobody gets back the console it was started from.
- `swaylock` started by hand, or any other lock screen, cannot lock the session: only Super + L
  and the triggers of the compositor do, with our program. A program of the session cannot
  show a fake lock screen through the protocol.
- The lock program exits cleanly only after PAM accepted the password: the toolkit
  (`exwlshellev`) sends `unlock_and_destroy` on every clean exit, so an error must never end it
  cleanly (it aborts, and the compositor keeps the session locked).
- Within the session, a process of the user can still trace the lock program
  (`kernel.yama.ptrace_scope` at 0 on Debian): the distribution will set it to 1.
- The protocol is applied by smithay plus our checks: a new version of smithay that changes
  `session_lock` must be read again against this ADR.
- The author tries the lock on the reference laptop before the merge (rule 9): killing the
  lock screen, plugging a screen in, switching console, going to sleep.

## Amendment of lot D-25j (2026-11-27, proposed: the author approves it in pull request)

The security audit of 27/11 (point 1) found that Ctrl + Alt + Backspace while locked, kept by
the decision of 2026-11-26, does not only close the session: started from a text console (the
documented way before greetd), the compositor gives back that console, already logged in, and
the lock is bypassed; a crash of the compositor does the same. Under greetd, anyone at the
machine could also close the session and lose the user's work.

- While locked, Ctrl + Alt + Backspace is not a shortcut: it goes to the lock surface as a
  Backspace. The console switch stays, and so does the rescue screen's Enter. Unlocked, it still
  stops the compositor, the way out when nothing answers.
- The compositor creates a file when the session locks and removes it at the unlock: the one
  `SHERLOCKOS_LOCK_MARK` names, `$XDG_RUNTIME_DIR/sherlockos-locked` in `sherlockos-session`.
  If the compositor ends, however it ends, while that file is there, `sherlockos-session` ends
  the logind session (`loginctl terminate-session`), or without one kills the process that
  started it, instead of giving back its console.
- A process of the user could remove that file, or the compositor: it already runs as the user,
  in the session; this protects the session from someone at the keyboard, not from the user's
  own programs (see the point on `ptrace` above).

## What would change our mind

- A crash of the compositor itself ends the session, and with it the lock: that is the case of
  every Wayland session. A watchdog that restarts the compositor locked would be a lot of its
  own.
