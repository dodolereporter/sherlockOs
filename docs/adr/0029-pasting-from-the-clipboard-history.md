# ADR 0029: Pasting from the clipboard history puts the item back, the user pastes

- Status: proposed
- Date: 2026-11-27
- Lot: D-42

## Context

Super + V opens the clipboard history (lot D-42): a panel of the shell, a layer-shell surface that
takes the keyboard while it is open. The user picks an item and presses Enter. The plan leaves
open what happens then:

- **A. Put back, then a virtual Ctrl + V.** The compositor makes the item the clipboard, the
  panel closes, the keyboard goes back to the window that had it, and the compositor sends that
  window a Ctrl + V it makes up, as Windows does with Win + V.
- **B. Put back only.** The compositor makes the item the clipboard, the panel closes, the
  keyboard goes back to the window that had it, and the user presses Ctrl + V (or uses the
  application's menu), as Maccy does by default on macOS.

What weighs against A:

- **Ctrl + V is not "paste" everywhere.** In a terminal (Konsole, our future Inquiry, any shell in
  a terminal), Ctrl + V inserts the next character literally, or is a shortcut of the program
  that runs in it (Vim's block selection); pasting is Ctrl + Shift + V. In other programs Ctrl + V
  is rebound, or does nothing where the cursor is. A made-up key press can thus type something
  the user did not want, in a window the user is not looking at.
- **The keys are made up by the compositor.** smithay sends them as if they came from the
  keyboard: the application cannot tell them apart. The compositor so far only forwards what the
  user presses (with its own shortcuts taken out), and the security model of the session (ADR
  0021, ADR 0023) relies on no client being able to make the compositor type. A request of the
  shell protocol that types into another window is that capability, even if only the shell
  socket offers it.
- **The target may have changed.** Between Enter and the made-up key press, the panel must
  close, its surface go, and the keyboard come back to the right window. A window that opens or
  takes the focus in between (a notification with the keyboard, a dialog) would receive the
  paste.
- **X11 windows** receive keys through XWayland with their own keymap state: a made-up Ctrl
  must be pressed and released in the right order, with the right modifiers state, or the X11
  application keeps Ctrl held.

What weighs against B: one more key press, and a new user expects Enter to paste, as on Windows.

## Decision

**B.** Enter (or a click) on an item asks the compositor, through the `paste` request of the
shell's clipboard protocol, to put the item back on the clipboard; the panel closes, the
keyboard goes back to the window that had it, and its footer says so beforehand: « Entrée pour
copier, puis Ctrl + V pour coller ». The item moves to the top of the history.

Only the clients of the shell socket see the clipboard protocol (ADR 0021): no application can
read the history nor put an item back.

## Consequences

- No key is ever made up by the compositor; nothing can be typed into the wrong window.
- The item is a real selection of the compositor, offered to Wayland and X11 applications alike
  (the bridge of lot D-29), in every type it was read as (text, files, picture).
- One key press more than Windows.

## What would change our mind

If the author wants Enter to paste directly, A can come later as an option of Control Room, off
by default, under these conditions: the compositor, not the panel, sends the keys; only to the
window that had the keyboard when the panel opened, and only if it has it again within a short
delay; Ctrl + Shift + V for the terminals we know (by app_id), nothing for an unknown terminal;
never while locked. That option would need its own ADR.
