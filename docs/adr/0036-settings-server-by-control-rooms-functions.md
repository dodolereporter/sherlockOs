# ADR 0036: The settings server, by Control Room's own functions

- Status: proposed
- Date: 2026-11-28
- Lot: A-17

## Context

The plan's criterion for A-17 is that every setting of the interface can also be driven by the
agent. Control Room already writes the settings files that the compositor, the shell and the
applications follow live, and drives the sound, Wi-Fi and Bluetooth through `system-services`. A
second writer would drift from it: another format, another validation, a file half written.

## Decision

**The same functions**: Control Room's settings module becomes the `settings-files` crate,
without iced, used by Control Room and by Sherlock's settings server. A key is set by putting its
line in the file's text in memory, asking the file's own parser whether it understands it, and
writing with `Settings::update`: only the keys that change, through the one safe writer of
settings files. The sound, Wi-Fi and Bluetooth go through `system-services`, which asks polkit
where its policy wants it.

**A table, not a free key**: the server offers the keys of a fixed table, each with what it
takes. A setting that is a path (a wallpaper's file, the lock's background) is not offered: a
path is for the files server and its folders. The screens are read only: they change live with
the compositor (ADR 0025), from Control Room.

**Levels**: reading is a read; changing a setting is an action; forgetting a Wi-Fi network is
destructive, since its password goes with it.

## Consequences

- A setting changed by Sherlock looks, on disk and in Control Room, exactly like one changed by
  hand; Control Room shows it at once, as for any change made elsewhere.
- A new setting of Control Room needs a line in the server's table to be driven by the agent.
- Joining a Wi-Fi network (a password) is not offered: secrets do not go through the model.

## What would make us reconsider

- The screens driven by the agent: a client of the compositor's output management in the
  server, with its own ADR.
