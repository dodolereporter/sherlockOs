//! The login screen of SherlockOS (lot D-24, ADR 0026), a greeter of greetd: greetd runs PAM and
//! starts the session, this program asks who logs in and relays the conversation. It runs as
//! greetd's user, in the compositor started with `--greeter`. See `docs/greeter.md`.
//!
//! - `greetd`: the client of greetd's IPC;
//! - `login`: a whole login, from the user name to the session started.

pub mod greetd;
pub mod login;
