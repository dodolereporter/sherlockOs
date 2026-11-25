//! User name, at the top of the Sherlock menu.

/// Full name of `login` in a file in the `/etc/passwd` format: the first GECOS field, or
/// `None` if it is empty.
pub fn display_name(passwd: &str, login: &str) -> Option<String> {
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        if fields.next()? != login {
            return None;
        }
        let gecos = fields.nth(3)?;
        let name = gecos.split(',').next()?.trim();
        (!name.is_empty()).then(|| name.to_owned())
    })
}

/// Name of the session user: their full name, otherwise their login.
pub fn current() -> String {
    let login = std::env::var("USER").unwrap_or_default();
    std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|passwd| display_name(&passwd, &login))
        .unwrap_or(login)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\n\
        dodo:x:1000:1000:Dodo,,,:/home/dodo:/bin/zsh\n\
        noname:x:1001:1001:,,,:/home/noname:/bin/sh\n";

    #[test]
    fn the_name_comes_from_gecos() {
        assert_eq!(display_name(PASSWD, "dodo").as_deref(), Some("Dodo"));
        assert_eq!(display_name(PASSWD, "noname"), None);
        assert_eq!(display_name(PASSWD, "unknown"), None);
    }
}
