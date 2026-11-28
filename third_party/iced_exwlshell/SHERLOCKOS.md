# iced_exwlshell 0.21.4, patched for SherlockOS

This is the published crate `iced_exwlshell` 0.21.4 (MIT, `LICENSE`, from
https://github.com/waycrate/exwlshelleventloop), used through `[patch.crates-io]` in the root
`Cargo.toml`. One change only, proposed upstream:

- `IcedNewPopupSettings::grab` (and its builder `grab(bool)`), `true` by default. With `false`,
  `NewPopUp` creates the popup without `xdg_popup.grab`, so that it leaves the keyboard to its
  parent: Evidence's completion list under the path field, and its tooltips (lot E-02c).

Files changed: `src/actions.rs` (with a test), `src/window.rs`. Everything else is the published crate as is.
Remove this folder and the `[patch.crates-io]` entry once a release carries the option.
