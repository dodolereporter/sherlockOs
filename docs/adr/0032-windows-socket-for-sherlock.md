# ADR 0032: The windows of the session, for Sherlock, through a private socket of the compositor

- Status: proposed
- Date: 2026-11-28
- Lot: A-12

## Context

Milestone J4 wants Sherlock to list, move and arrange the windows. The compositor holds them;
`sherlock_shell_v1` gives the shell their list and a few actions, but it is a Wayland protocol
reserved to the shell's programs (ADR 0019), and `sherlockd` is not a Wayland client. Whatever
gives Sherlock this power must stay behind the consent broker (ADR 0031): the model may obey an
instruction hidden in a window title, a page or a file, and a client of the session may try to
reach the power directly.

## Decision

**A private socket of the compositor, next to the consent one**: `<Wayland socket>-sherlock`,
one JSON object per line, answered at once. Only `sherlockd` of the same user connects: the
peer's uid (`SO_PEERCRED`) and its executable (`/proc/<pid>/exe`), as on the consent socket.
It is not opened on the login screen, and nothing acts while the session is locked.

**A small fixed set**: list; focus, maximize, minimize, snap left or right, move to a workspace
or a screen, arrange the windows shown; close. No geometry in pixels, no input sent to a
window, no access to its content. Every name is validated against what exists: an unknown
window, workspace, screen or side is refused, never guessed.

**The compositor does not ask for consent itself**: the windows server is a mode of
`sherlockd` (`sherlockd --windows-server <socket>`), spawned by `sherlockd` as one of its own
MCP servers (`ServerConfig::ours`), so its levels come from its annotations: the list is a
read, the actions an action, the closing destructive (the application may lose what is not
saved). Each action reaches it only through `McpServers::call`, with a `Permit`.

**What it reads is bounded**: a line of 4 KiB at most; a title and an application id on one
line, without control characters, cut to 200 characters. The tools' descriptions tell the
model that a title is the application's text and never an instruction.

## Consequences

- Sherlock arranges the windows with the gestures the user already has (Super + arrows, the
  dock's click, Mission Control's workspaces): nothing a window could not already be made to do.
- A process of the same user that runs `sherlockd` itself, or loads a library into it, passes
  the peer check of both sockets, as for the consent socket: the peer check guards against the
  session's other clients, not against code already running as the user.
- The shell's protocol stays the shell's: the two may diverge, the socket following what the
  windows server needs.

## What would make us reconsider

- A need for more than this set (a window's geometry, its content): a new ADR, each power
  weighed against the consent it would ask.
- A sandbox for the applications (Flatpak, portals), which would let the compositor check a
  stronger identity than an executable's name.
