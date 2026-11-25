//! Sources de l'état du système. Chacune implémente `Source` ; `Fixed` en est la fausse
//! implémentation des tests. `watch` fait tourner une source dans son propre fil et en tire un
//! flux d'états, qui ne contient que les changements.

pub mod battery;
pub mod clock;
pub mod network;
pub mod sound;
pub mod workspaces;

use std::fmt::Debug;
use std::io::{BufRead, BufReader, Lines};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Duration;

use iced::futures::Stream;
use iced::futures::channel::mpsc;

/// Délai avant de relancer une commande de surveillance qui s'est arrêtée.
const MONITOR_RETRY: Duration = Duration::from_secs(30);

/// Erreur d'une source.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("la commande {command} n'a pas pu être lancée : {source}")]
    Spawn {
        command: &'static str,
        source: std::io::Error,
    },
    #[error("la commande {command} a échoué : {stderr}")]
    Failed {
        command: &'static str,
        stderr: String,
    },
    #[error("sortie de {command} illisible : {detail}")]
    Parse {
        command: &'static str,
        detail: String,
    },
}

/// Une source d'état du système.
pub trait Source: Send + 'static {
    type State: Clone + PartialEq + Send + Debug + 'static;

    /// Lit l'état courant.
    fn read(&mut self) -> Result<Self::State, SourceError>;

    /// Attend que l'état ait pu changer. Rend `false` quand il ne changera plus.
    fn wait(&mut self) -> bool;
}

/// Fausse source : un état fixe, qui ne change jamais.
#[derive(Debug, Clone)]
pub struct Fixed<S>(pub S);

impl<S: Clone + PartialEq + Send + Debug + 'static> Source for Fixed<S> {
    type State = S;

    fn read(&mut self) -> Result<S, SourceError> {
        Ok(self.0.clone())
    }

    fn wait(&mut self) -> bool {
        false
    }
}

/// Fait tourner `source` dans un fil : lit, envoie l'état s'il a changé, attend, recommence.
/// Le fil s'arrête quand le flux est abandonné ou quand la source ne changera plus.
pub fn watch<S: Source>(mut source: S, name: &'static str) -> impl Stream<Item = S::State> {
    let (sender, receiver) = mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name(format!("source-{name}"))
        .spawn(move || {
            let mut last = None;
            loop {
                match source.read() {
                    Ok(state) if last.as_ref() != Some(&state) => {
                        if sender.unbounded_send(state.clone()).is_err() {
                            return;
                        }
                        last = Some(state);
                    }
                    Ok(_) => {}
                    Err(error) => tracing::warn!(source = name, %error, "lecture impossible"),
                }
                if !source.wait() {
                    return;
                }
            }
        });
    if let Err(error) = spawned {
        tracing::error!(source = name, %error, "fil de la source impossible à lancer");
    }
    receiver
}

/// Lance `program` avec `args` dans la langue C, pour une sortie stable, et rend sa sortie.
pub fn run(program: &'static str, args: &[&str]) -> Result<String, SourceError> {
    let output = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(|source| SourceError::Spawn {
            command: program,
            source,
        })?;
    if !output.status.success() {
        return Err(SourceError::Failed {
            command: program,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Une commande qui écrit une ligne à chaque changement, comme `upower --monitor`.
pub struct Monitor {
    program: &'static str,
    args: &'static [&'static str],
    /// Les lignes qui annoncent un changement utile.
    relevant: fn(&str) -> bool,
    running: Option<(Child, Lines<BufReader<ChildStdout>>)>,
}

impl Monitor {
    pub fn new(
        program: &'static str,
        args: &'static [&'static str],
        relevant: fn(&str) -> bool,
    ) -> Self {
        Self {
            program,
            args,
            relevant,
            running: None,
        }
    }

    /// Attend la prochaine ligne utile. Si la commande s'arrête, la relance après un délai.
    pub fn wait(&mut self) -> bool {
        loop {
            let Some((_, lines)) = &mut self.running else {
                match self.start() {
                    Ok(running) => self.running = Some(running),
                    Err(error) => {
                        tracing::warn!(command = self.program, %error, "surveillance impossible");
                        std::thread::sleep(MONITOR_RETRY);
                        return true;
                    }
                }
                continue;
            };
            match lines.next() {
                Some(Ok(line)) if (self.relevant)(&line) => return true,
                Some(Ok(_)) => {}
                Some(Err(_)) | None => {
                    self.stop();
                    tracing::warn!(command = self.program, "surveillance arrêtée, relance");
                    std::thread::sleep(MONITOR_RETRY);
                    return true;
                }
            }
        }
    }

    fn start(&self) -> std::io::Result<(Child, Lines<BufReader<ChildStdout>>)> {
        let mut child = Command::new(self.program)
            .args(self.args)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("sortie standard absente"))?;
        Ok((child, BufReader::new(stdout).lines()))
    }

    fn stop(&mut self) {
        if let Some((mut child, _)) = self.running.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop();
    }
}

/// La valeur d'une ligne `clé: valeur` dans une sortie, espaces retirés.
fn field<'a>(output: &'a str, key: &str) -> Option<&'a str> {
    output.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        (k.trim() == key).then(|| v.trim())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::StreamExt;
    use iced::futures::executor::block_on;

    #[test]
    fn une_source_fixe_rend_son_etat_une_fois() {
        let states: Vec<_> = block_on(watch(Fixed(42), "test").collect());
        assert_eq!(states, [42]);
    }

    /// Une source qui rend une suite d'états, puis s'arrête.
    struct Sequence(Vec<u8>);

    impl Source for Sequence {
        type State = u8;

        fn read(&mut self) -> Result<u8, SourceError> {
            self.0.first().copied().ok_or(SourceError::Parse {
                command: "test",
                detail: String::from("vide"),
            })
        }

        fn wait(&mut self) -> bool {
            self.0.remove(0);
            !self.0.is_empty()
        }
    }

    #[test]
    fn seuls_les_changements_sont_envoyes() {
        let states: Vec<_> = block_on(watch(Sequence(vec![1, 1, 2, 2, 1]), "test").collect());
        assert_eq!(states, [1, 2, 1]);
    }

    #[test]
    fn un_champ_se_lit_par_sa_cle() {
        let output = "  state:   charging\n  percentage:  55%\n";
        assert_eq!(field(output, "state"), Some("charging"));
        assert_eq!(field(output, "percentage"), Some("55%"));
        assert_eq!(field(output, "energy"), None);
    }
}
