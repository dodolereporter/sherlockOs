//! Connecting to a file server of the network without root (lot E-07): through gvfs, with
//! `gio mount`, which mounts it in the user's FUSE folder (`$XDG_RUNTIME_DIR/gvfs`). Every call
//! runs off the interface thread, with a time limit; a missing `gio` gives a clear message.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use sherlock_ui::texts::evidence as texts;

/// Longest a mount or an unmount may take (a slow server, a password check).
const TIMEOUT: Duration = Duration::from_secs(30);
/// Most of a command's output kept.
const MAX_OUTPUT: u64 = 64 * 1024;

/// How a connection may fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// gvfs's `gio` is not installed.
    NoGio,
    /// The server wants a name and a password.
    Credentials,
    /// It did not answer in time.
    Timeout,
    /// Anything else, with what `gio` said (its first line).
    Failed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoGio => f.write_str(texts::NETWORK_NO_GIO),
            Self::Credentials => f.write_str(texts::NETWORK_CREDENTIALS),
            Self::Timeout => f.write_str(texts::NETWORK_TIMEOUT),
            Self::Failed(why) => f.write_str(&texts::network_failed(why)),
        }
    }
}

/// A name and a password for a server.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub user: String,
    pub domain: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    /// Never the password in a log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Credentials({}@{})", self.user, self.domain)
    }
}

/// Runs `gio` with `args`, writing `input` to it, for `TIMEOUT` at most: its exit status and
/// what it wrote (standard output, then errors).
fn gio(args: &[&str], input: &str) -> Result<(bool, String), Error> {
    let mut child = Command::new("gio")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => Error::NoGio,
            _ => Error::Failed(error.to_string()),
        })?;
    if let Some(mut stdin) = child.stdin.take() {
        // Closed after: a question gio asks beyond these lines reads the end of the input.
        let _ = stdin.write_all(input.as_bytes());
    }
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Timeout);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(error) => return Err(Error::Failed(error.to_string())),
        }
    };
    let mut text = String::new();
    for pipe in [
        child.stdout.take().map(|p| Box::new(p) as Box<dyn Read>),
        child.stderr.take().map(|p| Box::new(p) as Box<dyn Read>),
    ]
    .into_iter()
    .flatten()
    {
        let mut part = String::new();
        let _ = pipe.take(MAX_OUTPUT).read_to_string(&mut part);
        text.push_str(&part);
    }
    Ok((status.success(), text))
}

/// Whether what `gio mount` said means the server wants a name and a password.
fn wants_credentials(output: &str) -> bool {
    let lower = output.to_lowercase();
    // The words differ with the version of gvfs and of Samba: a question gio could not ask
    // (its input is closed), or the refusal itself, in its words or as an NT status.
    [
        "password",
        "authentication",
        "permission denied",
        "logon failure",
        "access denied",
        "not permitted",
        "user [",
        "anonymous",
        "nt_status_access",
        "nt_status_logon",
        "nt_status_wrong_password",
        "unauthorized",
    ]
    .iter()
    .any(|word| lower.contains(word))
}

/// The FUSE folder of a mounted address, from `gio info`.
fn local_path(uri: &str) -> Result<PathBuf, Error> {
    let (_, text) = gio(&["info", "-a", "standard::name", uri], "")?;
    text.lines()
        .find_map(|line| line.strip_prefix("local path: "))
        .map(|path| PathBuf::from(path.trim()))
        .ok_or_else(|| Error::Failed(texts::NETWORK_NO_FOLDER.into()))
}

/// Mounts `uri` (`smb://host/` or `smb://host/share/`): as a guest without `credentials`,
/// else with them. Returns the folder that shows it.
pub fn mount(uri: &str, credentials: Option<&Credentials>) -> Result<PathBuf, Error> {
    let (ok, output) = match credentials {
        None => gio(&["mount", "--anonymous", uri], "")?,
        // gio asks, in this order, for the user, the domain and the password.
        Some(c) => gio(
            &["mount", uri],
            &format!("{}\n{}\n{}\n", c.user, c.domain, c.password),
        )?,
    };
    let already = output.to_lowercase().contains("already mounted");
    if !ok && !already {
        if wants_credentials(&output) {
            return Err(Error::Credentials);
        }
        let first = output.lines().next().unwrap_or_default();
        return Err(Error::Failed(crate::pathbar::visible(first).into_owned()));
    }
    local_path(uri)
}

/// Unmounts `uri`.
pub fn unmount(uri: &str) -> Result<(), Error> {
    match gio(&["mount", "--unmount", uri], "")? {
        (true, _) => Ok(()),
        (false, output) => {
            let first = output.lines().next().unwrap_or_default();
            Err(Error::Failed(crate::pathbar::visible(first).into_owned()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gio_messages_are_read() {
        assert!(wants_credentials(
            "gio: smb://nas/: Password dialog cancelled"
        ));
        assert!(wants_credentials("Failed to mount: Permission denied"));
        assert!(!wants_credentials("gio: smb://nas/: Host is down"));
        // Other versions: the question it could not ask, an NT status, an errno.
        assert!(wants_credentials(
            "Authentication Required\nEnter user and password for share “nas”:\nUser [ada]: "
        ));
        assert!(wants_credentials(
            "gio: smb://nas/: Failed to mount Windows share: NT_STATUS_ACCESS_DENIED"
        ));
        assert!(wants_credentials(
            "gio: smb://nas/: Failed to mount Windows share: Operation not permitted"
        ));
        assert!(wants_credentials(
            "gio: smb://nas/: Anonymous access denied"
        ));
        assert!(!wants_credentials(
            "gio: smb://nas/: Failed to mount Windows share: Connection refused"
        ));
        let c = Credentials {
            user: "ada".into(),
            domain: "WORKGROUP".into(),
            password: "secret".into(),
        };
        assert!(!format!("{c:?}").contains("secret"));
    }
}
