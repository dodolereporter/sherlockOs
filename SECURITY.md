# Security policy

## Reporting a vulnerability

**Do not open a public issue.** Report it privately through GitHub:
[Report a vulnerability](https://github.com/dodolereporter/sherlockOs/security/advisories/new)
(the "Security" tab of the repository, then "Report a vulnerability").

Say what is affected (component, commit), how to reproduce it, and what an attacker gains. The
project is maintained in spare time: the answer comes in the advisory, as soon as possible. The fix is prepared in a private advisory, then published with
credit to you, unless you prefer not to be named.

## Scope

SherlockOS is young and has no release yet: only the `main` branch is supported.

The parts where a flaw matters most:

| Component | What to look for |
| --- | --- |
| Lock screen (`desktop/lock`, `ext-session-lock-v1` in the compositor, ADR 0023) | Anything shown or clickable while locked, unlocking without the password, a crash that leaves the session open, the password kept in memory or logs |
| Login (`--greeter` mode, greetd client) | The same, before a session is open |
| Compositor (`desktop/compositor`) | Wayland and XWayland requests that crash it, read other clients' content or input, or escape the restrictions of the private protocols (`sherlock-shell-v1`, session lock); unbounded reads of files or buffers |
| `sherlockd` and the consent broker (`sherlock/`) | An action run without the user's consent, instructions hidden in data that change the agent's behaviour, secrets sent to the model, the journal altered |
| Shell and applications | Unbounded or unvalidated external input: files, D-Bus, desktop entries, icons |

Out of scope: flaws in third-party programs (report them upstream), and attacks that need root
or physical access to an unlocked session.
