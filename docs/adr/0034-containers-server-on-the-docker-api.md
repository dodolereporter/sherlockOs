# ADR 0034: The containers server, a minimal client of the Docker Engine API

- Status: proposed
- Date: 2026-11-28
- Lot: A-14

## Context

ADR 0009 ships Docker Engine and Compose, and puts the containers server on the Docker API. The
exit criterion of milestone J4 is the scenario « relance front-next »: Sherlock notices that a
container of the project is stopped, proposes to restart it, gets the consent, restarts it and
journals it. Whoever reaches the Docker socket is root on the machine, or close to it: what
Sherlock may ask of it must be small, checked and bounded.

## Decision

**The API, not the command**: a client of our own, in `sherlockd`, for a dozen requests of the
Engine API over its Unix socket, one HTTP/1.1 request per connection. No `docker` or `docker
compose` command, no shell, no new dependency (no HTTP crate for so little). Podman's
compatible socket is used when Docker has none.

**A fixed set**: list containers, images and Compose projects (from their labels), a container's
state and logs (reads); start, stop, restart a container or a Compose service (actions); remove
a container, an image, a volume, never forced (destructive). No creation, no `exec`, no image
pulled, no environment read.

**Checked and bounded**: every name is matched against what Docker accepts before it goes into a
path; the times, the header lines and the body are bounded; the socket's peer must be root or the
user. What is read is data, given as JSON after a sentence that says so, with what looks like a
secret masked in the logs.

## Consequences

- The scenario « relance front-next » runs through the broker: one consent for the restart,
  journaled with its outcome.
- A Compose service is acted on container by container: a service that is not created yet
  (`docker compose up`) is out of reach, as is a project's network.
- Masking secrets in logs is a best effort on common shapes; the model may still read a secret a
  container prints in another form.

## What would make us reconsider

- A need for `exec` or for building images: the sandboxes (ADR 0010, L-12) come first.
- An API version the daemons of the distribution no longer serve.
