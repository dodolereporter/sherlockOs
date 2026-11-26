//! The bar's D-Bus connections, opened on first use and then shared: the session bus for the
//! notification daemon. And `coalesce`, which calms the signals the panels follow. They follow `DBUS_SESSION_BUS_ADDRESS`, which is how the tests point
//! the bar at a private bus.

use std::sync::OnceLock;
use std::time::Duration;

use iced::futures::stream::{self, BoxStream};
use iced::futures::{FutureExt, StreamExt};
use zbus::Connection;

/// Longest wait for an answer on the session bus: a daemon that hangs gives an error, never a
/// panel that waits for ever.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// The session bus.
pub async fn session() -> zbus::Result<Connection> {
    static BUS: OnceLock<Connection> = OnceLock::new();
    if let Some(bus) = BUS.get() {
        return Ok(bus.clone());
    }
    let bus = zbus::connection::Builder::session()?
        .method_timeout(TIMEOUT)
        .build()
        .await?;
    Ok(BUS.get_or_init(|| bus).clone())
}

/// Least time between two changes `coalesce` lets through.
pub const SIGNAL_GAP: Duration = Duration::from_millis(150);

/// The changes a service announces, at most one batch every `gap`, each change once in it: a
/// burst of signals (a Bluetooth scan seeing the same devices again and again, a Wi-Fi scan)
/// gives one read and one redraw, never one per signal. The first change waits `gap` too, for
/// the rest of its burst.
pub fn coalesce<T>(changes: BoxStream<'static, T>, gap: Duration) -> BoxStream<'static, T>
where
    T: PartialEq + Send + 'static,
{
    let batches = stream::unfold(changes.ready_chunks(64), move |mut chunks| async move {
        let mut batch = chunks.next().await?;
        pause(gap).await;
        while let Some(Some(more)) = chunks.next().now_or_never() {
            batch.extend(more);
        }
        let mut once: Vec<T> = Vec::new();
        for change in batch {
            if !once.contains(&change) {
                once.push(change);
            }
        }
        Some((stream::iter(once), chunks))
    });
    batches.flatten().boxed()
}

/// Waits `gap` on a thread of its own: the bar has no timer of an async runtime.
async fn pause(gap: Duration) {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name(String::from("bar-signals"))
        .spawn(move || {
            std::thread::sleep(gap);
            let _ = sender.send(());
        });
    match spawned {
        Ok(_) => {
            let _ = receiver.await;
        }
        Err(error) => tracing::warn!(%error, "cannot wait between two signals"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::executor::block_on;

    #[test]
    fn a_burst_of_signals_gives_each_change_once() {
        let burst = stream::iter([1, 2, 1, 1, 2, 3, 1]).boxed();
        let changes: Vec<_> = block_on(coalesce(burst, Duration::from_millis(5)).collect());
        assert_eq!(changes, [1, 2, 3]);
        let start = std::time::Instant::now();
        let none: Vec<u8> = block_on(coalesce(stream::empty::<u8>().boxed(), SIGNAL_GAP).collect());
        assert!(none.is_empty() && start.elapsed() < SIGNAL_GAP);
    }
}
