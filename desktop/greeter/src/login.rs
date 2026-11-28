//! A whole login through greetd: the session is created for the user, PAM's prompts are
//! answered, its messages kept for the screen, then the chosen session is started. PAM runs in
//! greetd (its `greetd` service): this side only relays the conversation.
//!
//! The password answers the first secret prompt, once. A second secret prompt (a password to
//! change, a second factor) is not handled yet: the login is given up with PAM's question,
//! which the screen shows. So is a visible prompt, since the user name is already given.
//!
//! greetd starts the session once its greeter has left (`greetd-ipc(7)`): the screen plays its
//! leaving animation, then ends, and the compositor of the login screen with it.

use system_services::secret::Secret;

use crate::greetd::{Error, Greetd, Prompt, Reply};

/// The session to start: its command and what goes into its environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub command: Vec<String>,
    pub env: Vec<String>,
}

/// A message PAM gave on the way, to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub text: String,
    /// An error rather than an information.
    pub error: bool,
}

/// How the login ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Authenticated, and the session will start once this greeter leaves.
    Started,
    /// The password was refused, or the user is unknown: PAM does not tell them apart.
    Refused,
    /// PAM asked something the screen cannot answer yet: its question.
    Unsupported(String),
    /// greetd could not do it: its reason (the session did not start, greetd is busy…).
    Failed(String),
}

/// What a login gives back: how it ended, and PAM's messages on the way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub outcome: Outcome,
    pub notes: Vec<Note>,
}

/// Logs `user` in with `password`, then starts `launch`. The password is wiped once sent, and
/// whatever happens, nothing is left half done in greetd: a login given up is cancelled.
pub fn log_in(
    greetd: &mut Greetd,
    user: &str,
    mut password: Secret,
    launch: &Launch,
) -> Result<Report, Error> {
    let mut notes = Vec::new();
    let mut reply = greetd.create_session(user)?;
    let mut answered = false;
    let outcome = loop {
        reply = match reply {
            Reply::AuthMessage(Prompt::Secret, question) => {
                if answered {
                    break give_up(greetd, question)?;
                }
                answered = true;
                let reply = greetd.answer(Some(&password));
                password.wipe();
                reply?
            }
            Reply::AuthMessage(Prompt::Visible, question) => break give_up(greetd, question)?,
            Reply::AuthMessage(kind @ (Prompt::Info | Prompt::Error), text) => {
                notes.push(Note {
                    text,
                    error: kind == Prompt::Error,
                });
                greetd.answer(None)?
            }
            // The session is cancelled after an error, as greetd's own greeters do: greetd
            // may still hold it, and the next attempt creates a new one.
            Reply::AuthError(_) => {
                greetd.cancel_session()?;
                break Outcome::Refused;
            }
            Reply::Error(reason) => {
                greetd.cancel_session()?;
                break Outcome::Failed(reason);
            }
            Reply::Success => match greetd.start_session(&launch.command, &launch.env)? {
                Reply::Success => break Outcome::Started,
                Reply::AuthError(reason) | Reply::Error(reason) => {
                    greetd.cancel_session()?;
                    break Outcome::Failed(reason);
                }
                Reply::AuthMessage(_, text) => {
                    greetd.cancel_session()?;
                    break Outcome::Failed(text);
                }
            },
        };
    };
    password.wipe();
    Ok(Report { outcome, notes })
}

fn give_up(greetd: &mut Greetd, question: String) -> Result<Outcome, Error> {
    greetd.cancel_session()?;
    Ok(Outcome::Unsupported(question))
}
