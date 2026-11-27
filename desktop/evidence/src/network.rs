//! The file servers of the local network (lot E-07), found without any tool installed: a
//! multicast DNS question for `_smb._tcp.local` (Samba, macOS, NAS boxes, through Avahi or
//! Bonjour), and a WS-Discovery probe (Windows shares). Both run on a thread of their own, for
//! `DISCOVERY_TIME` at most, read at most `MAX_PACKETS` answers of `MAX_PACKET` bytes, and keep
//! at most `MAX_SERVERS`. No iced here.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

/// How long the network is listened to.
pub const DISCOVERY_TIME: Duration = Duration::from_millis(2500);
/// Answers read at most, their largest size, and servers kept at most.
const MAX_PACKETS: usize = 256;
const MAX_PACKET: usize = 9000;
pub const MAX_SERVERS: usize = 64;
/// Longest name kept for a server.
const MAX_NAME: usize = 63;

const MDNS: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(224, 0, 0, 251), 5353);
const WSD: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 3702);
const SMB_SERVICE: &str = "_smb._tcp.local";

/// A file server of the network.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Server {
    /// The name it announces (« NAS du salon »), else its address.
    pub name: String,
    /// What `smb://` reaches it by: its `.local` host name, else its address.
    pub host: String,
}

impl Server {
    /// Its address for gvfs.
    pub fn uri(&self) -> String {
        format!("smb://{}/", self.host)
    }
}

/// The servers that answer now, sorted by name; empty when the network cannot be reached.
pub fn discover() -> Vec<Server> {
    let mut found = BTreeMap::new();
    let Ok(socket) = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)) else {
        return Vec::new();
    };
    // Answers come back to this socket: a « one-shot » mDNS question (RFC 6762, 5.1), and the
    // WS-Discovery probe.
    let _ = socket.set_multicast_ttl_v4(1);
    let _ = socket.send_to(&mdns_question(SMB_SERVICE), MDNS);
    let _ = socket.send_to(wsd_probe().as_bytes(), WSD);
    let start = Instant::now();
    let mut buffer = vec![0u8; MAX_PACKET];
    for _ in 0..MAX_PACKETS {
        let Some(left) = DISCOVERY_TIME.checked_sub(start.elapsed()) else {
            break;
        };
        if socket
            .set_read_timeout(Some(left.max(Duration::from_millis(1))))
            .is_err()
        {
            break;
        }
        let Ok((size, from)) = socket.recv_from(&mut buffer) else {
            break;
        };
        let packet = &buffer[..size];
        let servers = if from.port() == MDNS.port() {
            mdns_servers(packet)
        } else {
            wsd_servers(packet, &from.ip().to_string())
        };
        for server in servers {
            if found.len() < MAX_SERVERS {
                found.entry(server.host.clone()).or_insert(server);
            }
        }
    }
    let mut servers: Vec<Server> = found.into_values().collect();
    servers.sort_by_key(|s| s.name.to_lowercase());
    servers
}

/// How often the network is listened to again while Evidence runs.
pub const REDISCOVER: Duration = Duration::from_secs(120);

/// The servers of the network, now and then every `REDISCOVER`, from a thread of its own.
pub fn stream() -> impl iced::futures::Stream<Item = Vec<Server>> {
    use iced::futures::{SinkExt, StreamExt};
    iced::stream::channel(
        4,
        |mut output: iced::futures::channel::mpsc::Sender<Vec<Server>>| async move {
            let (sender, mut receiver) = iced::futures::channel::mpsc::unbounded();
            let spawned = std::thread::Builder::new()
                .name("evidence-network".into())
                .spawn(move || {
                    while sender.unbounded_send(discover()).is_ok() {
                        std::thread::sleep(REDISCOVER);
                    }
                });
            if spawned.is_err() {
                return;
            }
            while let Some(servers) = receiver.next().await {
                if output.send(servers).await.is_err() {
                    break;
                }
            }
        },
    )
}

/// A DNS question for the PTR records of `service`, asking for a unicast answer.
fn mdns_question(service: &str) -> Vec<u8> {
    // Header: id 0, flags 0, one question.
    let mut packet = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in service.split('.') {
        packet.push(label.len() as u8);
        packet.extend(label.as_bytes());
    }
    // Type PTR (12), class IN with the unicast-response bit.
    packet.extend([0, 0, 0, 12, 0x80, 1]);
    packet
}

/// Reads a name at `at`, following compression pointers (a few at most), and where the name
/// ends in the packet.
fn read_name(packet: &[u8], mut at: usize) -> Option<(String, usize)> {
    let mut labels: Vec<String> = Vec::new();
    let mut end = None;
    for _ in 0..32 {
        let length = *packet.get(at)? as usize;
        match length {
            0 => {
                return Some((labels.join("."), end.unwrap_or(at + 1)));
            }
            l if l & 0xC0 == 0xC0 => {
                let pointer = ((l & 0x3F) << 8) | *packet.get(at + 1)? as usize;
                end.get_or_insert(at + 2);
                at = pointer;
            }
            l => {
                let label = packet.get(at + 1..at + 1 + l)?;
                labels.push(String::from_utf8_lossy(label).into_owned());
                at += 1 + l;
            }
        }
    }
    None
}

/// The servers an mDNS answer names: each SMB instance (PTR), its host (SRV), its name.
fn mdns_servers(packet: &[u8]) -> Vec<Server> {
    let read16 = |at: usize| -> Option<usize> {
        Some(u16::from_be_bytes([*packet.get(at)?, *packet.get(at + 1)?]) as usize)
    };
    let Some(header) = (0..6).map(|i| read16(2 * i)).collect::<Option<Vec<_>>>() else {
        return Vec::new();
    };
    let questions = header[2];
    let records = header[3] + header[4] + header[5];
    let mut at = 12;
    for _ in 0..questions {
        let Some((_, end)) = read_name(packet, at) else {
            return Vec::new();
        };
        at = end + 4;
    }
    let mut instances = Vec::new();
    let mut hosts: BTreeMap<String, String> = BTreeMap::new();
    for _ in 0..records.min(64) {
        let Some((owner, end)) = read_name(packet, at) else {
            break;
        };
        let (Some(kind), Some(length)) = (read16(end), read16(end + 8)) else {
            break;
        };
        let data = end + 10;
        if packet.len() < data + length {
            break;
        }
        match kind {
            // PTR: an instance of the service.
            12 if owner.eq_ignore_ascii_case(SMB_SERVICE) => {
                if let Some((instance, _)) = read_name(packet, data) {
                    instances.push(instance);
                }
            }
            // SRV: the host of an instance, after priority, weight and port.
            33 => {
                if let Some((host, _)) = read_name(packet, data + 6) {
                    hosts.insert(owner.to_lowercase(), host);
                }
            }
            _ => {}
        }
        at = data + length;
    }
    instances
        .into_iter()
        .filter_map(|instance| {
            let host = hosts.get(&instance.to_lowercase())?.clone();
            let name = instance
                .strip_suffix(&format!(".{SMB_SERVICE}"))
                .unwrap_or(&instance);
            Some(server(name, &host))
        })
        .collect()
}

/// A WS-Discovery probe for devices, as Windows' file explorer sends it.
fn wsd_probe() -> String {
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="utf-8"?>"#,
            r#"<soap:Envelope xmlns:soap="http://www.w3.org/2003/05/soap-envelope" "#,
            r#"xmlns:wsa="http://schemas.xmlsoap.org/ws/2004/08/addressing" "#,
            r#"xmlns:wsd="http://schemas.xmlsoap.org/ws/2005/04/discovery" "#,
            r#"xmlns:wsdp="http://schemas.xmlsoap.org/ws/2006/02/devprof">"#,
            r#"<soap:Header><wsa:To>urn:schemas-xmlsoap-org:ws:2005:04:discovery</wsa:To>"#,
            r#"<wsa:Action>http://schemas.xmlsoap.org/ws/2005/04/discovery/Probe</wsa:Action>"#,
            r#"<wsa:MessageID>urn:uuid:{:08x}-0000-4000-8000-000000000000</wsa:MessageID>"#,
            r#"</soap:Header><soap:Body><wsd:Probe><wsd:Types>wsdp:Device</wsd:Types>"#,
            r#"</wsd:Probe></soap:Body></soap:Envelope>"#
        ),
        std::process::id()
    )
}

/// The server a WS-Discovery answer comes from: a Windows computer or a Samba box with wsdd.
/// Only answers that are probe matches count; its address names it.
fn wsd_servers(packet: &[u8], from: &str) -> Vec<Server> {
    let text = String::from_utf8_lossy(packet);
    if !text.contains("ProbeMatches") || !text.contains("wsdp:Device") {
        return Vec::new();
    }
    vec![server(from, from)]
}

/// A server, its name cut to `MAX_NAME` characters and safe to show.
fn server(name: &str, host: &str) -> Server {
    let name: String = crate::pathbar::visible(name)
        .chars()
        .take(MAX_NAME)
        .collect();
    let host: String = host
        .trim_end_matches('.')
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_'))
        .take(253)
        .collect();
    Server { name, host }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name in wire format.
    fn name(text: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for label in text.split('.') {
            out.push(label.len() as u8);
            out.extend(label.as_bytes());
        }
        out.push(0);
        out
    }

    fn record(owner: &[u8], kind: u16, data: &[u8]) -> Vec<u8> {
        let mut out = owner.to_vec();
        out.extend(kind.to_be_bytes());
        out.extend([0, 1, 0, 0, 0, 120]);
        out.extend((data.len() as u16).to_be_bytes());
        out.extend(data);
        out
    }

    #[test]
    fn an_mdns_answer_names_its_servers() {
        let mut packet = vec![0, 0, 0x84, 0, 0, 0, 0, 2, 0, 0, 0, 0];
        // PTR _smb._tcp.local -> NAS du salon._smb._tcp.local, the service name compressed.
        let service = packet.len();
        packet.extend(record(
            &name("_smb._tcp.local"),
            12,
            &[
                name("NAS du salon")
                    .split_last()
                    .map(|(_, l)| l.to_vec())
                    .unwrap_or_default(),
                vec![0xC0, service as u8],
            ]
            .concat(),
        ));
        // SRV for that instance -> nas.local, port 445.
        let mut srv = vec![0, 0, 0, 0, 1, 189];
        srv.extend(name("nas.local"));
        packet.extend(record(&name("NAS du salon._smb._tcp.local"), 33, &srv));
        let servers = mdns_servers(&packet);
        assert_eq!(
            servers,
            [Server {
                name: "NAS du salon".into(),
                host: "nas.local".into()
            }]
        );
        assert_eq!(servers[0].uri(), "smb://nas.local/");
    }

    #[test]
    fn broken_or_looping_answers_name_nothing() {
        assert!(mdns_servers(&[0, 1, 2]).is_empty());
        // A pointer to itself never ends: refused.
        let mut packet = vec![0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0];
        packet.extend([0xC0, 12]);
        assert!(mdns_servers(&packet).is_empty());
        assert!(read_name(&[0xC0, 0], 0).is_none());
    }

    #[test]
    fn a_wsd_match_names_its_address_and_names_are_cleaned() {
        let answer = b"<soap:Envelope><wsd:ProbeMatches><wsd:Types>wsdp:Device pub:Computer</wsd:Types></wsd:ProbeMatches></soap:Envelope>";
        assert_eq!(
            wsd_servers(answer, "192.168.1.20"),
            [Server {
                name: "192.168.1.20".into(),
                host: "192.168.1.20".into()
            }]
        );
        assert!(wsd_servers(b"<Probe/>", "192.168.1.20").is_empty());
        let odd = server("Disque\nmalin", "nas.local./x y");
        assert_eq!(
            (odd.name.as_str(), odd.host.as_str()),
            ("Disque␊malin", "nas.local.xy")
        );
        assert!(mdns_question(SMB_SERVICE).ends_with(&[0, 0, 12, 0x80, 1]));
        assert!(wsd_probe().contains("wsdp:Device"));
    }
}
