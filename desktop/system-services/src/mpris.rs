//! Media players through MPRIS, on the session bus: play or pause, next and previous track. The
//! command goes to the player that plays, or else to one that is paused, or else to the first
//! one, by name.

use zbus::Connection;
use zbus::names::OwnedBusName;
use zbus::zvariant::OwnedValue;

use crate::{Problem, Result};

/// Every player owns a name under this prefix.
pub const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

/// What a media key asks of the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    PlayPause,
    Pause,
    Next,
    Previous,
}

impl Command {
    /// The method of `org.mpris.MediaPlayer2.Player`.
    pub fn method(self) -> &'static str {
        match self {
            Self::PlayPause => "PlayPause",
            Self::Pause => "Pause",
            Self::Next => "Next",
            Self::Previous => "Previous",
        }
    }
}

/// The player a command goes to, among `players` with their `PlaybackStatus`: the first that
/// plays, else the first that is paused, else the first.
pub fn choose(players: &[(String, String)]) -> Option<&str> {
    ["Playing", "Paused"]
        .iter()
        .find_map(|wanted| players.iter().find(|(_, status)| status == wanted))
        .or(players.first())
        .map(|(name, _)| name.as_str())
}

/// Sends `command` to the player of the session that should get it. Returns its bus name.
pub async fn send(bus: &Connection, command: Command) -> Result<String> {
    let dbus = zbus::fdo::DBusProxy::new(bus).await?;
    let mut players = Vec::new();
    for name in dbus.list_names().await? {
        let name: OwnedBusName = name;
        if !name.as_str().starts_with(PREFIX) {
            continue;
        }
        let status = bus
            .call_method(
                Some(name.as_str()),
                PATH,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(PLAYER, "PlaybackStatus"),
            )
            .await
            .ok()
            .and_then(|reply| reply.body().deserialize::<OwnedValue>().ok())
            .and_then(|value| String::try_from(value).ok())
            .unwrap_or_default();
        players.push((name.to_string(), status));
    }
    players.sort();
    let Some(player) = choose(&players).map(str::to_owned) else {
        return Err(Problem::Absent(String::from("no media player")));
    };
    bus.call_method(
        Some(player.as_str()),
        PATH,
        Some(PLAYER),
        command.method(),
        &(),
    )
    .await?;
    Ok(player)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn players(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(n, s)| ((*n).to_owned(), (*s).to_owned()))
            .collect()
    }

    #[test]
    fn the_player_that_plays_gets_the_key() {
        let list = players(&[("a", "Stopped"), ("b", "Paused"), ("c", "Playing")]);
        assert_eq!(choose(&list), Some("c"));
        let list = players(&[("a", "Stopped"), ("b", "Paused")]);
        assert_eq!(choose(&list), Some("b"));
        let list = players(&[("a", "Stopped"), ("b", "")]);
        assert_eq!(choose(&list), Some("a"));
        assert_eq!(choose(&[]), None);
    }
}
