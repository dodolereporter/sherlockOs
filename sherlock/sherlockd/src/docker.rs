//! A minimal client of the Docker Engine API (lot A-14, ADR 0034), over its Unix socket: one
//! HTTP/1.1 request per connection, no shell, no command line. Podman's compatible socket
//! answers the same requests.
//!
//! Everything is bounded: the time to connect, write and read; the status line and the
//! headers; the body (`Content-Length` or chunked). The socket's peer must be root (the system
//! daemon) or this user (rootless Docker, Podman): a socket another user planted is not asked.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The API version asked for: Docker 20.10 and later, Podman's compatible API.
pub const API: &str = "/v1.41";
/// Bytes of a response body at most.
pub const MAX_BODY: usize = 4 * 1024 * 1024;
/// Bytes of a header line, headers of a response, at most.
const MAX_LINE: usize = 8 * 1024;
const MAX_HEADERS: usize = 64;

/// The daemon, at its socket.
#[derive(Debug, Clone)]
pub struct Docker {
    socket: PathBuf,
    timeout: Duration,
}

/// A response: its status and its body.
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Docker {
    pub fn at(socket: &Path, timeout: Duration) -> Self {
        Self {
            socket: socket.to_owned(),
            timeout,
        }
    }

    /// The session's daemon: rootless Docker (`$XDG_RUNTIME_DIR/docker.sock`), the system's
    /// (`/var/run/docker.sock`), else Podman's (`$XDG_RUNTIME_DIR/podman/podman.sock`).
    pub fn locate() -> Option<PathBuf> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
        let runtime = runtime.filter(|r| r.is_absolute());
        let mut places = Vec::new();
        if let Some(runtime) = &runtime {
            places.push(runtime.join("docker.sock"));
        }
        places.push(PathBuf::from("/var/run/docker.sock"));
        if let Some(runtime) = &runtime {
            places.push(runtime.join("podman/podman.sock"));
        }
        places.into_iter().find(|p| p.exists())
    }

    /// `method` on `path` (under [`API`]), without a body. `Err` is said to the model.
    pub fn request(&self, method: &str, path: &str) -> Result<Response, String> {
        let unreachable = || String::from("le démon Docker est injoignable");
        let mut stream = UnixStream::connect(&self.socket).map_err(|_| unreachable())?;
        if !trusted(&stream) {
            tracing::error!(socket = ?self.socket, "docker socket held by another user");
            return Err(unreachable());
        }
        stream.set_read_timeout(Some(self.timeout)).ok();
        stream.set_write_timeout(Some(self.timeout)).ok();
        let head = format!(
            "{method} {API}{path} HTTP/1.1\r\nHost: docker\r\nUser-Agent: sherlockd\r\n\
             Connection: close\r\nContent-Length: 0\r\n\r\n"
        );
        stream
            .write_all(head.as_bytes())
            .map_err(|_| unreachable())?;
        read_response(BufReader::new(stream)).map_err(|error| {
            tracing::warn!(error, path, "docker response refused");
            format!("réponse du démon Docker illisible : {error}")
        })
    }
}

/// Whether the peer of `stream` is root or this user.
fn trusted(stream: &UnixStream) -> bool {
    let Ok(peer) = rustix::net::sockopt::socket_peercred(stream) else {
        return false;
    };
    peer.uid.is_root() || peer.uid == rustix::process::getuid()
}

/// One line, CRLF or LF, of `MAX_LINE` bytes at most.
fn line(reader: &mut impl BufRead) -> Result<String, &'static str> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_LINE as u64 + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| "lecture interrompue")?;
    if bytes.len() > MAX_LINE || !bytes.ends_with(b"\n") {
        return Err("ligne trop longue ou coupée");
    }
    let text = String::from_utf8(bytes).map_err(|_| "pas du texte")?;
    Ok(text.trim_end_matches(['\r', '\n']).to_owned())
}

/// Reads a response: its status line, its headers, its body within [`MAX_BODY`].
pub fn read_response(mut reader: impl BufRead) -> Result<Response, &'static str> {
    let status = line(&mut reader)?;
    let mut parts = status.splitn(3, ' ');
    let (Some(version), Some(code)) = (parts.next(), parts.next()) else {
        return Err("pas de statut");
    };
    if !version.starts_with("HTTP/1.") {
        return Err("pas du HTTP/1");
    }
    let status: u16 = code.parse().map_err(|_| "statut invalide")?;
    let (mut length, mut chunked) = (None, false);
    for n in 0.. {
        if n == MAX_HEADERS {
            return Err("trop d'en-têtes");
        }
        let header = line(&mut reader)?;
        if header.is_empty() {
            break;
        }
        let Some((name, value)) = header.split_once(':') else {
            return Err("en-tête invalide");
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            let n: usize = value.parse().map_err(|_| "longueur invalide")?;
            if n > MAX_BODY {
                return Err("réponse trop grande");
            }
            length = Some(n);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            chunked = value.eq_ignore_ascii_case("chunked");
        }
    }
    let mut body = Vec::new();
    if chunked {
        loop {
            let size = line(&mut reader)?;
            let size = size.split(';').next().unwrap_or("").trim();
            if size.len() > 8 {
                return Err("morceau trop grand");
            }
            let size = usize::from_str_radix(size, 16).map_err(|_| "morceau invalide")?;
            if size == 0 {
                break;
            }
            if body.len() + size > MAX_BODY {
                return Err("réponse trop grande");
            }
            let start = body.len();
            body.resize(start + size, 0);
            reader
                .read_exact(&mut body[start..])
                .map_err(|_| "morceau coupé")?;
            line(&mut reader)?;
        }
    } else if let Some(length) = length {
        body.resize(length, 0);
        reader.read_exact(&mut body).map_err(|_| "réponse coupée")?;
    } else if status != 204 && status != 304 {
        reader
            .take(MAX_BODY as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|_| "réponse coupée")?;
        if body.len() > MAX_BODY {
            return Err("réponse trop grande");
        }
    }
    Ok(Response { status, body })
}

/// The lines of a container's logs: the multiplexed stream of a container without a terminal
/// (8-byte headers: stream, 0, 0, 0, length), or the raw one of a container with one.
pub fn log_text(body: &[u8]) -> String {
    let multiplexed = body.len() >= 8 && body[0] <= 2 && body[1..4] == [0, 0, 0];
    if !multiplexed {
        return String::from_utf8_lossy(body).into_owned();
    }
    let mut text = Vec::new();
    let mut rest = body;
    while rest.len() >= 8 {
        let size = u32::from_be_bytes([rest[4], rest[5], rest[6], rest[7]]) as usize;
        let end = (8 + size).min(rest.len());
        text.extend_from_slice(&rest[8..end]);
        rest = &rest[end..];
    }
    String::from_utf8_lossy(&text).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &[u8]) -> Result<Response, &'static str> {
        read_response(BufReader::new(text))
    }

    #[test]
    fn responses_are_read_within_their_bounds() {
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
        assert_eq!(read(chunked).unwrap().body, b"abcde");
        let plain = b"HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\n\r\n{}";
        let response = read(plain).unwrap();
        assert_eq!(
            (response.status, response.body.as_slice()),
            (404, &b"{}"[..])
        );
        assert_eq!(read(b"HTTP/1.1 204 No Content\r\n\r\n").unwrap().body, b"");
        let huge = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY + 1
        );
        assert!(read(huge.as_bytes()).is_err());
        let chunk = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nFFFFFFFFF\r\n";
        assert!(read(chunk).is_err());
        let long = format!("HTTP/1.1 200 OK\r\nX: {}\r\n\r\n", "a".repeat(MAX_LINE));
        assert!(read(long.as_bytes()).is_err());
        assert!(read(b"SSH-2.0-OpenSSH\r\n").is_err());
        assert!(read(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc").is_err());
    }

    #[test]
    fn multiplexed_logs_lose_their_headers() {
        let mut body = vec![1, 0, 0, 0, 0, 0, 0, 3];
        body.extend_from_slice(b"ab\n");
        body.extend_from_slice(&[2, 0, 0, 0, 0, 0, 0, 2]);
        body.extend_from_slice(b"c\n");
        assert_eq!(log_text(&body), "ab\nc\n");
        assert_eq!(log_text(b"raw\n"), "raw\n");
    }
}
