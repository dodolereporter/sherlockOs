# ADR 0022: The name of the OS is SherlockOS

- Status: accepted (the author chose the name on 2026-11-26)
- Date: 2026-11-26
- Lot: Q-10

## Context

Until now the OS had no name. CLAUDE.md asked to write the placeholder `NOM_OS` wherever one
was needed, and the code followed it in several spellings: `NOM_OS` in environment variables
and in what the session shows, `nom-os` in paths and theme names, `nom_os` in the dconf
database of the session, `NOMOS` in the KDE color schemes, `org.nomos` in a D-Bus interface.

The author has chosen the name: SherlockOS.

## Decision

The OS is called **SherlockOS**. The placeholder rule of CLAUDE.md goes away. The spellings
are:

| Where | Spelling |
| --- | --- |
| Shown to the user (session name, theme names, texts) | `SherlockOS` |
| `XDG_CURRENT_DESKTOP`, `XDG_SESSION_DESKTOP` | `SherlockOS` |
| Paths, files, folders, theme and dconf names | `sherlockos` (e.g. `~/.local/state/sherlockos`, cursor themes `sherlockos` and `sherlockos-light`) |
| Environment variables | `SHERLOCKOS_*` (e.g. `SHERLOCKOS_BIN`, `SHERLOCKOS_SESSION_BUS`) |
| D-Bus names | `org.sherlockos.*` |
| GTK themes and KDE color schemes | `SherlockOS-dark`, `SherlockOS-light`, `SherlockOSDark`, `SherlockOSLight` |

No compatibility layer: nothing under the old names was installed outside the repository
(the themes, the session and the cursors were never installed on the author's machine), except
the session log in `~/.local/state/nom-os`, which can be deleted.

ADRs 0000 to 0016 and `docs/design/` are records and keep the old name.

## Consequences

- Every new text, path or variable uses the spellings above.
- The mascot and the name will need a check of rights before any public release (the
  question was already open for the mascot).
