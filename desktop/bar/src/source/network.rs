//! Network, read from NetworkManager with the `nmcli` command. The OS network service is not
//! chosen yet (batch S-13): another `Source` implementation will replace this one if needed.

use super::{Monitor, Source, SourceError, run};

/// The link that carries the connection.
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

/// Splits a `nmcli -t` line, where `\:` is a colon inside a value.
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

/// Parses the output of `nmcli -t -f TYPE,STATE,CONNECTION device`. Only Wi-Fi and Ethernet
/// devices count: Docker bridges or the loopback are not a connection.
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

/// The machine's network, watched with `nmcli monitor`.
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
    fn wifi_connected() {
        // Recorded on the reference laptop: Wi-Fi, Docker bridges, Ethernet unplugged.
        let network = parse(include_str!("../../fixtures/nmcli-wifi.txt"));
        assert_eq!(
            network,
            Network::Connected {
                link: Link::Wifi,
                name: String::from("MonWifi")
            }
        );
    }

    #[test]
    fn ethernet_connected() {
        let network = parse(include_str!("../../fixtures/nmcli-wired.txt"));
        assert!(matches!(
            network,
            Network::Connected {
                link: Link::Wired,
                ..
            }
        ));
    }

    #[test]
    fn bridges_do_not_count() {
        let network = parse(include_str!("../../fixtures/nmcli-disconnected.txt"));
        assert_eq!(network, Network::Disconnected);
        assert_eq!(parse(""), Network::Disconnected);
    }

    #[test]
    fn an_escaped_colon_stays_in_the_name() {
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
