# ADR 0023: Security model of the session lock

- Status: proposed (to be approved by the author before the merge of lot D-25a1)
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
   - Ctrl + Alt + Backspace, which stops the compositor: it closes the session, it does not
     open it;
   - the volume and brightness keys (not the players);
   - Enter on the rescue screen (below).
   Power off and sleep through the power button belong to logind, not to the compositor.
4. **Only the lock that holds the session ends it.** The requests of `ext_session_lock_v1` go
   through our own dispatch before smithay's: `unlock_and_destroy` is refused with
   `invalid_unlock` (a protocol error, which disconnects the client) unless it comes from the
   lock the compositor accepted and confirmed. A surface created on a lock that was refused is
   never shown nor given input.
5. **Who may lock.** While the session is unlocked, any client may lock it, as the protocol
   wants (that is how `swaylock` works). While it is locked, a second lock is refused
   (`finished`), and once the lock has no holder (its client died), only a process the
   compositor started itself, recognized by the pid of its connection (`SO_PEERCRED`), may take
   it over.
6. **The client dies, the session stays locked.** If the lock object goes away without
   `unlock_and_destroy` (crash, kill, disconnection), the session stays locked on the plain
   background, and the compositor starts the lock program again (`sherlockos-lock`), after a
   pause of 500 ms. A program that ends without locking, or does not lock within 10 seconds,
   counts as a failure. After 3 failures in a row, it starts `swaylock` on the same background
   color; if that fails too (not installed), it draws a rescue screen itself: « Verrouillé »,
   and Enter to try the programs again. The counter starts again at each unlock.
7. **The password** is checked by PAM only, in the lock program, for the user of the session
   only (lot D-25a2). The compositor never sees it.

## Consequences

- A crash of the lock screen never shows the desktop: at worst the user sees the rescue
  screen, and gets out of it with Enter, another console, or Ctrl + Alt + Backspace, which ends
  the session.
- Any program of the session can still lock an unlocked session and show something of its
  own as a lock screen (a fake lock screen that asks for the password). The protocol allows it;
  restricting the global to the programs the compositor starts would close it, at the price of
  `swaylock` started by hand. Left for later.
- The protocol is applied by smithay plus our checks: a new version of smithay that changes
  `session_lock` must be read again against this ADR.
- The author tries the lock on the reference laptop before the merge (rule 9): killing the
  lock screen, plugging a screen in, switching console, going to sleep.

## What would change our mind

- A crash of the compositor itself ends the session, and with it the lock: that is the case of
  every Wayland session. A watchdog that restarts the compositor locked would be a lot of its
  own.
- If the fake lock screen above becomes a real concern, the `ext_session_lock_manager_v1`
  global would be shown only to the processes the compositor starts.
