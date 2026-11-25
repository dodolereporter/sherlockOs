//! Nom de l'utilisateur, en tête du menu Sherlock.

/// Nom complet de `login` dans un fichier au format de `/etc/passwd` : le premier champ du
/// GECOS, ou `None` s'il est vide.
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

/// Nom de l'utilisateur de la session : son nom complet, sinon son identifiant.
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
        sansnom:x:1001:1001:,,,:/home/sansnom:/bin/sh\n";

    #[test]
    fn le_nom_vient_du_gecos() {
        assert_eq!(display_name(PASSWD, "dodo").as_deref(), Some("Dodo"));
        assert_eq!(display_name(PASSWD, "sansnom"), None);
        assert_eq!(display_name(PASSWD, "inconnu"), None);
    }
}
