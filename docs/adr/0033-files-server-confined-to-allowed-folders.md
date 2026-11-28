# ADR 0033: The files server, confined to the allowed folders by the kernel

- Status: proposed
- Date: 2026-11-28
- Lot: A-13

## Context

Milestone J4 wants Sherlock to read and arrange the user's files, and the plan's criterion for
A-13 is that an access out of an allowed folder is refused and journaled. What attacks it: the
model, obeying an instruction hidden in a file it read; a file planted in an allowed folder (a
symbolic link to `~/.ssh`, a pipe that never ends, a name that breaks a line); a race between a
check of a path and its use.

## Decision

**Folders, not paths**: a settings file lists the allowed folders (by default the documents,
desktop and downloads folders). A folder is refused if it holds or lies in a place of secrets
(`~/.ssh`, `~/.gnupg`, the keyrings, Claude Code's files, Sherlock's own settings, journal and
data): `~` and `/` are never folders. The server cannot change its own settings: they are in
`~/.config/sherlock`.

**The kernel confines**: each folder is opened once, at start; every path is opened from that
descriptor with `openat2(RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS)` (through `rustix`, no
`unsafe`), and every change acts with the `*at` calls on a held descriptor of the parent, on a
last name that is never followed. No canonicalisation then check: there is no window between the
two. A kernel without `openat2` reaches nothing.

**Levels**: listing, reading and searching are reads; creating, writing, renaming, moving,
copying and trashing are actions (the trash can be undone); deleting for good is destructive,
only for a file or an empty folder.

**Data, never instructions**: what is read is given as JSON after a sentence that says so, every
hidden character escaped; only regular files are read, bounded.

**Journaled**: the broker's journal gets, besides the call and its decision, the error of a tool
that failed: a refused access is written with its reason.

## Consequences

- An access out of the folders is refused by the kernel even if the check of the path were wrong.
- A hard link in an allowed folder to a file elsewhere is read: making it needs the user's own
  hands, which could as well copy the file.
- A folder on another disk than the trash cannot be trashed from: refused, not copied.
- The settings have no window yet: the user edits `files.json` (a page of the settings, later).

## What would make us reconsider

- A sandbox for the servers (Landlock), which would confine the whole process as well.
- A need to follow mounts or links across folders.
