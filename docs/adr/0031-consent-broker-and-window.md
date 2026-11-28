# ADR 0031: The consent broker, its window drawn by the compositor, and the reinforced consent

- Status: proposed
- Date: 2026-11-28
- Lot: A-11

## Context

ADR 0011 puts a consent broker in front of every tool call of Sherlock, with the consent window
drawn by the compositor, and leaves the form of the reinforced consent of the destructive level
to this lot. ADR 0015 gives each tool its level; rule S2 (`docs/sherlockd.md`) makes a read that
reaches outside the machine an action. What attacks it: the model, obeying an instruction hidden
in a file, a page or a tool result; a Wayland client, which would imitate the window, click on it
or cover it; a broker that fails open when something goes wrong.

## Decision

**In `sherlockd`, one path.** `McpServers::call`, the only public way to a server's
`tools/call`, hands every call to the broker (`broker::Broker::authorize`) with its level. A read
runs at once. Anything else waits for the user's answer. Only a `Granted` answer gives a
`Permit`, which only the broker module can make, which names the tool, its level and the exact
arguments the user was shown, and which the call consumes. `agent::execute` keeps refusing
everything but read for a tool host that has no broker (`ToolHost::consents`), as defence in
depth.

**Refused by default.** Everything that is not a `Granted` answer refuses the call: the user
says no, closes the window, does not answer within 60 seconds (`CONSENT_TIMEOUT`), the compositor
is unreachable or goes away during the request, the journal cannot be written. Nothing retries
on its own.

**The journal.** `$XDG_STATE_HOME/sherlock/journal.jsonl`, created 0600, opened in append mode,
never truncated, never through a symbolic link. One JSON object per line: the decision of each
request (time, id, tool, level, arguments cut to 2 KiB, decision), then the outcome of each call
that ran (time, id, tool, ok, error or failed). JSON escapes every line break of a value: a tool
or the model cannot forge a line. A read whose decision cannot be journaled still runs; an action
does not.

**The window, drawn by the compositor** (lot A-11b). `sherlockd` asks the compositor through a
private socket of the compositor's (`$XDG_RUNTIME_DIR`, 0600, its peer checked with
`SO_PEERCRED`): the request id, the tool, its level and the summary of its arguments. The
compositor draws the window itself, above every surface, the anchored layers and their overlay
layer included: no client draws it, no client can draw above it. While it is shown, the pointer
and the keyboard go to it alone, as for Mission Control: no client receives a click or a key,
and no client can send one to it (Wayland has no such request; XWayland's synthetic events
reach only X11 windows). It names the tool in plain words with its arguments, and never shows
text a client chose as its own. Its answer goes back on the socket that asked; a request whose
socket closes is withdrawn, and the window with it.

**Against a click that was meant for something else.** The buttons of the window do not act
during the first 0.6 s after it appears: a click or a key already on its way when the window
showed up refuses nothing and grants nothing.

**The reinforced consent of the destructive level.** The « Autoriser » button must be held down
for 1.5 s, with a ring that fills meanwhile; released earlier, nothing happens. Enter does not
grant it; Escape refuses it. The window states what cannot be undone. An action-level consent is
a click, or Enter.

**The same user: what the broker protects against, and what it does not** (from the A-20
review). The broker protects the user against the model: against an instruction hidden in a
file, a page, a window title or a tool result, which the model may obey. It does not protect
the session against a malicious program already running as the same user. The check of each
private socket is the peer's uid (`SO_PEERCRED`) and the *name* of its executable
(`/proc/<pid>/exe`, `sherlockd` or `compositor`): any binary of the user renamed so passes it,
as the end-to-end test of A-11d does on purpose. Such a program can also start `sherlockd
--files-server` or `--windows-server` itself and call `tools/call` without any broker, preload a
library into `sherlockd` (`LD_PRELOAD`), trace it (`ptrace`), or rewrite the journal, which
belongs to the user. Neither `SO_PEERCRED` nor `/proc/<pid>/exe` can do better without separating
the users: that would take a service under another user, which this ADR does not decide. Later
hardening, short of it: compare `/proc/<pid>/exe` with the installed path (`/usr/bin/…`) rather
than its name, or hand `sherlockd` a socket inherited from the compositor instead of a path.

## Consequences

- No call of an action-level tool, from the agent loop or from Claude Code through the proxy,
  reaches a server without a consent the user gave for that call, with those arguments.
- Until the window exists (A-11b and A-11c), `sherlockd` has nobody to ask: actions stay refused,
  as before, and every decision is journaled.
- Each action costs the user an answer: the grouping of calls into one plan, consented once, is
  for the interface lot (A-19).
- The setting through which the user grants another level to a tool of a third-party server stays
  for later (ADR 0015).
- A program of the session that wants to act without the user's consent can: the consent
  window is a guard against the model, not against the user's own programs.

## What would make us reconsider

- An action appears in the journal without a granted decision before it.
- The compositor cannot keep the window above every surface, or a client reaches its input.
- The answer delay or the hold of the reinforced consent prove too short or too long in use.
- Sherlock must resist a malicious program of the session: a service under a user of its own,
  with its own ADR.
