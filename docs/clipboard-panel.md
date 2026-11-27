# Clipboard history panel (Super + V)

Lot D-42. Crate `desktop/clipboard-panel`, binary `sherlockos-clipboard`. What was copied, in
Wayland and X11 applications, as the compositor keeps it (see "Clipboard history" in
`docs/compositor.md`), in a glass panel in the middle of the screen, like Windows' Win + V and
macOS's Maccy or Raycast.

## Opening and closing

- Super + V (Alt + V in a nested compositor) starts `sherlockos-clipboard`, found in
  `$SHERLOCKOS_BIN` then in the `PATH`. Started again while open, it closes the open one (the
  launcher's single instance, `$XDG_RUNTIME_DIR/clipboard.lock`): the same keys open and close
  it. Nothing opens while the session is locked: the compositor takes no shortcut then.
- It opens with a fade and a light scale-up, and closes the opposite way, on the launcher's
  spring (`sherlock_ui::clipboard::OPEN_SPRING`).
- Escape, a click beside the panel, or Enter on an item close it.

## What it shows

- A search field that has the keyboard at once: typing filters the items, by every word,
  without case nor accents, in the preview and the application's name.
- The items: pinned first, then the newest. Each row has the thumbnail of a picture or the
  glyph of its kind (text, link, files), two lines of preview ending with « … » when cut, and
  the application it was copied from with the time (« il y a 3 min »). Files show their names.
- The selected row has its pin and its remove button; a pinned item always shows its pin.
- The footer says what Enter does, « Entrée pour copier, puis Ctrl + V pour coller », and holds
  « Tout effacer » (the pinned items stay), disabled when there is nothing to clear.
- Instead of the items: « Rien n'a encore été copié », « Aucun résultat », the history turned off
  (with where to turn it on), or a session without the history (another compositor).

## Keys and pointer

| Key | Action |
| --- | --- |
| Up, Down, the wheel | Move the selection; the rows follow it |
| Enter, a click | Put the item back on the clipboard and close (ADR 0029) |
| Delete | Remove the selected item |
| Escape | Close |

Enter never pastes by itself: the keyboard goes back to the window that had it, and the user
pastes there (ADR 0029, proposed).

## Screen sizes

The surface is the panel's width and the screen's whole height: from that height the panel
learns how many rows fit (5 at most, `rows_for_screen`), and sits in the middle. Drawn in
software, whose surfaces have no alpha, the surface then shrinks to the panel itself.

## How it talks to the compositor

`sherlock_clipboard_history_v1` (`desktop/shell-protocol/protocol/sherlock-clipboard-v1.xml`),
only on the shell socket (ADR 0021): a thread reads the items and their thumbnails (a PNG in a
file, 1 MiB at most) and hands the panel each consistent state at `done`; the requests (`paste`,
`set_pinned`, `remove`, `clear`) leave from the interface's thread without waiting.

## Tests

`cargo test -p clipboard-panel`: the order and the search (`items.rs`), the selection that
follows the keys and stays on its item when the history changes, the previews, the messages,
the thumbnail limits (`app.rs`, `history.rs`). The protocol itself is tested from the
compositor's side, in `desktop/compositor/tests/clipboard.rs`.
