//! Réseau, lu chez NetworkManager par la commande `nmcli`. Le service réseau de l'OS n'est pas
//! encore choisi (lot S-13) : une autre implémentation de `Source` le remplacera au besoin.

use super::{Monitor, Source, SourceError, run};

/// Le lien qui porte la connexion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Wifi,
    Wired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Network {
    Connected { link: Link, name: String },
    Disconnected,
}

/// Découpe une ligne de `nmcli -t`, où `\:` est un deux-points dans une valeur.
fn split_terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => fields
                .last_mut()
                .into_iter()
                .for_each(|f| f.extend(chars.next())),
            ':' => fields.push(String::new()),
            c => fields.last_mut().into_iter().for_each(|f| f.push(c)),
        }
    }
    fields
}

/// Lit la sortie de `nmcli -t -f TYPE,STATE,CONNECTION device`. Seules les cartes Wi-Fi et
/// Ethernet comptent : les ponts de Docker ou la boucle locale ne sont pas une connexion.
pub fn parse(output: &str) -> Network {
    output
        .lines()
        .find_map(|line| {
            let [kind, state, name] = <[String; 3]>::try_from(split_terse(line)).ok()?;
            let link = match kind.as_str() {
                "wifi" => Link::Wifi,
                "ethernet" => Link::Wired,
                _ => return None,
            };
            (state == "connected").then_some(Network::Connected { link, name })
        })
        .unwrap_or(Network::Disconnected)
}

/// Le réseau du poste, surveillé par `nmcli monitor`.
pub struct NetworkManager {
    monitor: Monitor,
}

impl Default for NetworkManager {
    fn default() -> Self {
        Self {
            monitor: Monitor::new("nmcli", &["monitor"], |_| true),
        }
    }
}

impl Source for NetworkManager {
    type State = Network;

    fn read(&mut self) -> Result<Network, SourceError> {
        let args = ["-t", "-f", "TYPE,STATE,CONNECTION", "device"];
        Ok(parse(&run("nmcli", &args)?))
    }

    fn wait(&mut self) -> bool {
        self.monitor.wait()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi_connecte() {
        // Enregistrée sur le portable de référence : Wi-Fi, ponts Docker, Ethernet débranché.
        let network = parse(include_str!("../../sorties/nmcli-wifi.txt"));
        assert_eq!(
            network,
            Network::Connected {
                link: Link::Wifi,
                name: String::from("CESI_HotSpot")
            }
        );
    }

    #[test]
    fn ethernet_connecte() {
        let network = parse(include_str!("../../sorties/nmcli-filaire.txt"));
        assert!(matches!(
            network,
            Network::Connected {
                link: Link::Wired,
                ..
            }
        ));
    }

    #[test]
    fn les_ponts_ne_comptent_pas() {
        let network = parse(include_str!("../../sorties/nmcli-deconnecte.txt"));
        assert_eq!(network, Network::Disconnected);
        assert_eq!(parse(""), Network::Disconnected);
    }

    #[test]
    fn un_deux_points_echappe_reste_dans_le_nom() {
        let network = parse("wifi:connected:Café\\:2\n");
        assert_eq!(
            network,
            Network::Connected {
                link: Link::Wifi,
                name: String::from("Café:2")
            }
        );
    }
}
