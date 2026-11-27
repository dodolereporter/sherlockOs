//! Who may log in: the human users of the machine. From the password database (`/etc/passwd`),
//! a user is human when their id is in the range of `/etc/login.defs` (`UID_MIN` to `UID_MAX`,
//! 1000 to 60000 by default) and their shell is a login shell: listed in `/etc/shells`, and not
//! `nologin` nor `false`. When AccountsService answers on the system bus, its accounts marked as
//! system accounts are left out and its real names are taken.
//!
//! Every file is read with a bound (`MAX_FILE`): a database that size is not a password
//! database. The screen shows the list of the files first; AccountsService is asked afterwards,
//! on a thread of its own (`accounts`), each call within `ACCOUNTS_TIMEOUT`, for
//! `MAX_ACCOUNTS` accounts at most: a slow or stuck service never holds the first frame.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The largest file read here.
pub const MAX_FILE: u64 = 4 * 1024 * 1024;

/// Longest wait for each answer of AccountsService.
pub const ACCOUNTS_TIMEOUT: Duration = Duration::from_secs(2);

/// Most accounts read from AccountsService.
pub const MAX_ACCOUNTS: usize = 64;

/// The ids of human users, when `/etc/login.defs` does not say.
pub const UID_MIN: u32 = 1000;
pub const UID_MAX: u32 = 60000;

/// A user of the machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// The login name.
    pub name: String,
    /// The full name, from the comment field or AccountsService, if any.
    pub real_name: Option<String>,
    pub uid: u32,
}

impl User {
    /// The name shown: the full name, or the login name.
    pub fn shown(&self) -> &str {
        self.real_name.as_deref().unwrap_or(&self.name)
    }

    /// The letter of the bubble: the first letter of the shown name, in capitals.
    pub fn initial(&self) -> String {
        self.shown()
            .chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_default()
    }
}

/// Reads a text file with the bound, `None` if it cannot.
pub fn read_bounded(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut text = String::new();
    file.take(MAX_FILE + 1).read_to_string(&mut text).ok()?;
    if text.len() as u64 > MAX_FILE {
        tracing::warn!(path = %path.display(), "too long: ignored");
        return None;
    }
    Some(text)
}

/// The range of human ids in the text of `login.defs`.
pub fn uid_range(login_defs: Option<&str>) -> (u32, u32) {
    let value = |key: &str| {
        login_defs?.lines().find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some(key)).then(|| words.next()?.parse().ok())?
        })
    };
    (
        value("UID_MIN").unwrap_or(UID_MIN),
        value("UID_MAX").unwrap_or(UID_MAX),
    )
}

/// The login shells in the text of `/etc/shells`; `None` if the file is missing, when any shell
/// but `nologin` and `false` counts.
pub fn login_shells(shells: Option<&str>) -> Option<Vec<String>> {
    Some(
        shells?
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with('/'))
            .map(str::to_owned)
            .collect(),
    )
}

fn is_login_shell(shell: &str, shells: Option<&[String]>) -> bool {
    let name = shell.rsplit('/').next().unwrap_or_default();
    if shell.is_empty() || matches!(name, "nologin" | "false") {
        return false;
    }
    shells.is_none_or(|shells| shells.iter().any(|s| s == shell))
}

/// The human users in the text of a password database, in its order.
pub fn from_passwd(passwd: &str, range: (u32, u32), shells: Option<&[String]>) -> Vec<User> {
    let mut users: Vec<User> = Vec::new();
    for line in passwd.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        let [name, _, uid, _, gecos, _, shell] = fields[..] else {
            continue;
        };
        let Ok(uid) = uid.parse::<u32>() else {
            continue;
        };
        if name.is_empty()
            || name.starts_with(['+', '-'])
            || !(range.0..=range.1).contains(&uid)
            || !is_login_shell(shell, shells)
            || users.iter().any(|u| u.name == name)
        {
            continue;
        }
        let real_name = gecos.split(',').next().map(str::trim).unwrap_or_default();
        users.push(User {
            name: name.to_owned(),
            real_name: (!real_name.is_empty()).then(|| real_name.to_owned()),
            uid,
        });
    }
    users
}

/// What AccountsService says of an account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    pub real_name: String,
    pub system: bool,
}

/// Leaves out the system accounts of AccountsService, and takes its real names.
pub fn merge(users: Vec<User>, accounts: &[Account]) -> Vec<User> {
    users
        .into_iter()
        .filter(|user| !accounts.iter().any(|a| a.name == user.name && a.system))
        .map(|mut user| {
            let account = accounts.iter().find(|a| a.name == user.name);
            if let Some(account) = account.filter(|a| !a.real_name.trim().is_empty()) {
                user.real_name = Some(account.real_name.trim().to_owned());
            }
            user
        })
        .collect()
}

/// The accounts of AccountsService, if it runs: its cached users, those that logged in once.
/// Blocks up to `ACCOUNTS_TIMEOUT` for each call: run it on a thread of its own.
pub fn accounts() -> Option<Vec<Account>> {
    match ask_accounts() {
        Ok(accounts) => Some(accounts),
        Err(err) => {
            tracing::debug!("AccountsService: {err}");
            None
        }
    }
}

fn ask_accounts() -> Result<Vec<Account>, zbus::Error> {
    const SERVICE: &str = "org.freedesktop.Accounts";
    let bus = zbus::blocking::connection::Builder::system()?
        .method_timeout(ACCOUNTS_TIMEOUT)
        .build()?;
    let manager = zbus::blocking::Proxy::new(
        &bus,
        SERVICE,
        "/org/freedesktop/Accounts",
        "org.freedesktop.Accounts",
    )?;
    let paths: Vec<zbus::zvariant::OwnedObjectPath> = manager.call("ListCachedUsers", &())?;
    let mut accounts = Vec::new();
    for path in paths.into_iter().take(MAX_ACCOUNTS) {
        let user = zbus::blocking::Proxy::new(
            &bus,
            SERVICE,
            path.as_str(),
            "org.freedesktop.Accounts.User",
        )?;
        accounts.push(Account {
            name: user.get_property("UserName")?,
            real_name: user.get_property("RealName").unwrap_or_default(),
            system: user.get_property("SystemAccount").unwrap_or(false),
        });
    }
    Ok(accounts)
}

/// The files the list is read from.
#[derive(Debug, Clone)]
pub struct Sources {
    pub passwd: PathBuf,
    pub login_defs: PathBuf,
    pub shells: PathBuf,
}

impl Default for Sources {
    fn default() -> Self {
        Self {
            passwd: "/etc/passwd".into(),
            login_defs: "/etc/login.defs".into(),
            shells: "/etc/shells".into(),
        }
    }
}

/// The human users of the machine, from the files only (AccountsService: `accounts`).
pub fn list(sources: &Sources) -> Vec<User> {
    let Some(passwd) = read_bounded(&sources.passwd) else {
        tracing::warn!(path = %sources.passwd.display(), "no password database");
        return Vec::new();
    };
    let range = uid_range(read_bounded(&sources.login_defs).as_deref());
    let shells = login_shells(read_bounded(&sources.shells).as_deref());
    from_passwd(&passwd, range, shells.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "\
root:x:0:0:root:/root:/bin/bash
daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin
_greetd:x:998:998::/var/lib/greetd:/usr/sbin/nologin
alex:x:1000:1000:Alex Martin,,,:/home/alex:/usr/bin/zsh
invite:x:1001:1001::/home/invite:/bin/bash
service:x:1002:1002::/srv:/usr/sbin/nologin
ferme:x:1003:1003::/home/ferme:/bin/false
nobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin
broken line
+nis::::::
alex:x:1000:1000:Twice:/home/alex:/usr/bin/zsh
";

    const SHELLS: &str = "# /etc/shells\n/bin/sh\n/bin/bash\n/usr/bin/bash\n/usr/bin/zsh\n";

    fn names(users: &[User]) -> Vec<&str> {
        users.iter().map(|u| u.name.as_str()).collect()
    }

    #[test]
    fn humans_are_the_users_with_a_login_shell_in_the_range() {
        let shells = login_shells(Some(SHELLS));
        let users = from_passwd(PASSWD, uid_range(None), shells.as_deref());
        assert_eq!(names(&users), ["alex", "invite"]);
        assert_eq!(users[0].real_name.as_deref(), Some("Alex Martin"));
        assert_eq!(
            (users[0].shown(), users[1].shown()),
            ("Alex Martin", "invite")
        );
        assert_eq!(
            (users[0].initial(), users[1].initial()),
            ("A".into(), "I".into())
        );
    }

    #[test]
    fn a_shell_missing_from_etc_shells_is_not_a_login_shell() {
        let shells = login_shells(Some("/bin/bash\n"));
        let users = from_passwd(PASSWD, uid_range(None), shells.as_deref());
        assert_eq!(names(&users), ["invite"]);
        // Without /etc/shells, any shell but nologin and false.
        let users = from_passwd(PASSWD, uid_range(None), None);
        assert_eq!(names(&users), ["alex", "invite"]);
    }

    #[test]
    fn the_range_comes_from_login_defs() {
        let defs = "# comment\nUID_MIN\t\t 1001\nUID_MAX 1001\n";
        assert_eq!(uid_range(Some(defs)), (1001, 1001));
        assert_eq!(uid_range(Some("UID_MIN nope")), (UID_MIN, UID_MAX));
        let users = from_passwd(PASSWD, uid_range(Some(defs)), None);
        assert_eq!(names(&users), ["invite"]);
    }

    #[test]
    fn accounts_service_hides_system_accounts_and_names_the_users() {
        let users = from_passwd(PASSWD, uid_range(None), None);
        let accounts = [
            Account {
                name: "invite".into(),
                real_name: "Invité".into(),
                system: false,
            },
            Account {
                name: "alex".into(),
                real_name: " ".into(),
                system: false,
            },
        ];
        let merged = merge(users.clone(), &accounts);
        assert_eq!(merged[0].shown(), "Alex Martin");
        assert_eq!(merged[1].shown(), "Invité");
        let hidden = [Account {
            name: "alex".into(),
            real_name: String::new(),
            system: true,
        }];
        assert_eq!(names(&merge(users, &hidden)), ["invite"]);
    }

    #[test]
    fn the_list_is_read_from_files_with_a_bound() {
        let dir = std::env::temp_dir().join(format!("greeter-users-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sources = Sources {
            passwd: dir.join("passwd"),
            login_defs: dir.join("login.defs"),
            shells: dir.join("shells"),
        };
        std::fs::write(&sources.passwd, PASSWD).unwrap();
        std::fs::write(&sources.shells, SHELLS).unwrap();
        assert_eq!(names(&list(&sources)), ["alex", "invite"]);
        // A directory, or a missing file, gives nobody rather than an error.
        let missing = Sources {
            passwd: dir.clone(),
            ..sources.clone()
        };
        assert!(list(&missing).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
