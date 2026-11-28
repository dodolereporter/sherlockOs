# ADR 0035: The services server, on systemd's D-Bus API and polkit

- Status: proposed
- Date: 2026-11-28
- Lot: A-16

## Context

The plan's criterion for A-16 is that Sherlock explains a service in failure from its logs.
Reading the state of a unit and its journal is harmless; starting, stopping or restarting a unit
of the system changes the machine for every user, and must never go through `sudo`.

## Decision

**systemd over D-Bus** (`org.freedesktop.systemd1`, with zbus, already in the desktop): the
units of the user on the session bus, those of the system on the system bus. No `systemctl`.

**The journal through `journalctl --output=json`**, with fixed arguments, the unit's name checked
and given after `--unit=`, a time limit and a bounded output. Reading journald's files ourselves
(or `sd-journal` through C) would add a format to follow, or `unsafe`, for no gain.

**Levels**: a unit of the user is an action; a unit of the system is destructive, the reinforced
consent, and goes with D-Bus's interactive authorization flag, so that systemd asks polkit,
which asks the user for a password where its policy wants one. Sherlock has no right of its
own: what polkit refuses stays refused.

**Data, never instructions**: states and journals go as JSON after a sentence that says so, every
hidden character escaped, what looks like a secret masked.

## Consequences

- An action on a system unit costs two answers: Sherlock's reinforced consent, then polkit's.
- A journal the user may not read (another user's, the system's without the `systemd-journal`
  group) gives nothing: `journalctl` reads what the user may read.
- Masking secrets is a best effort on common shapes.

## What would make us reconsider

- A need to follow a journal live, or to edit a unit: a new ADR.
