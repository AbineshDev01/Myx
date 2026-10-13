//! High-level Soloist client.
//!
//! Owns endpoint discovery, connect/reconnect with backoff, the
//! WebSocket read loop and command dispatch. The UI never touches
//! the protocol: it sends [`Command`]s through [`ClientHandle`]
//! and receives [`ClientEvent`]s.

use std::path::PathBuf;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use super::discovery;
use super::protocol::{Command, ServerEvent};

#[derive(Debug, Clone)]
pub struct ClientOptions {
    /// Explicit `host:port` (config). When set, discovery files
    /// are not consulted.
    pub explicit: Option<discovery::Endpoint>,
    /// Data directory used for `ws.addr` / `ws.port` discovery.
    pub data_dir: PathBuf,
    pub connect_timeout: Duration,
    pub reconnect_min: Duration,
    pub reconnect_max: Duration,
    /// Interval between protocol-level pings (dead-peer detection).
    pub ping_interval: Duration,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            explicit: None,
            data_dir: discovery::default_data_dir(),
            connect_timeout: Duration::from_secs(5),
            reconnect_min: Duration::from_millis(500),
            reconnect_max: Duration::from_secs(10),
            ping_interval: Duration::from_secs(30),
        }
    }
}

/// Everything the application needs to know about the link to
/// Soloist.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum ClientEvent {
    /// A connection attempt is starting (endpoint known or not).
    Connecting { endpoint: Option<String> },
    /// WebSocket handshake succeeded.
    Connected { endpoint: String },
    /// A parsed Soloist event arrived.
    Event(ServerEvent),
    /// The link is down. Automatic reconnection continues in the
    /// background.
    Disconnected { reason: String },
}

/// A handle that queues commands for the current connection.
/// Cheap to clone; the connection task owns the socket.
#[derive(Clone)]
pub struct ClientHandle {
    cmd_tx: flume::Sender<Command>,
}

impl ClientHandle {
    /// Queue a command for the current connection. Returns
    /// `false` when the connection task has exited (e.g. during
    /// shutdown).
    pub fn send(&self, cmd: Command) -> bool {
        self.cmd_tx.send(cmd).is_ok()
    }

    #[cfg(test)]
    pub fn dummy() -> (Self, flume::Receiver<Command>) {
        let (cmd_tx, rx) = flume::unbounded();
        (Self { cmd_tx }, rx)
    }
}


/// Spawn the connection supervisor. Emits events into `events`
/// until the process exits (reconnection is infinite by design —
/// Soloist restarting must never kill the player).
pub fn spawn(options: ClientOptions, events: flume::Sender<ClientEvent>) -> ClientHandle {
    let (cmd_tx, cmd_rx) = flume::unbounded();
    tokio::spawn(connection_loop(options, events, cmd_rx));
    ClientHandle { cmd_tx }
}

async fn connection_loop(
    options: ClientOptions,
    events: flume::Sender<ClientEvent>,
    mut cmd_rx: flume::Receiver<Command>,
) {
    let mut backoff = options.reconnect_min;
    // Set once a session has connected. Commands queued
    // before that — the startup ones (state, activate,
    // volume) — are legitimate and must survive failed
    // first attempts; only commands queued during a later
    // gap are stale.
    let connected_once = std::sync::atomic::AtomicBool::new(false);

    loop {
        let session_started = std::time::Instant::now();
        let endpoint = match resolve_endpoint(&options) {
            Ok(ep) => ep,
            Err(err) => {
                let reason = if discovery::daemon_running(&options.data_dir) {
                    format!("{err} (Soloist is running but its WebSocket API is disabled — restart it with --ws)")
                } else {
                    format!("{err} (no Soloist process found)")
                };
                let _ = events.send(ClientEvent::Disconnected { reason });
                tokio::time::sleep(backoff).await;
                backoff = next_backoff(backoff, &options);
                continue;
            }
        };

        let _ = events.send(ClientEvent::Connecting {
            endpoint: Some(endpoint.to_string()),
        });

        match run_session(&endpoint, &options, &events, &mut cmd_rx, &connected_once).await {
            Ok(()) => {
                let _ = events.send(ClientEvent::Disconnected {
                    reason: "connection closed".into(),
                });
            }
            Err(err) => {
                log::warn!(target: "myx::soloist", "soloist session ended (endpoint={endpoint}): {err}");
                let _ = events.send(ClientEvent::Disconnected { reason: err });
            }
        }

        // A server that accepts and immediately drops must not be
        // hammered: only sessions that survived a moment reset the
        // backoff.
        if session_started.elapsed() < Duration::from_secs(2) {
            backoff = next_backoff(backoff, &options);
        } else {
            backoff = options.reconnect_min;
        }
        tokio::time::sleep(backoff).await;
    }
}

fn resolve_endpoint(options: &ClientOptions) -> Result<discovery::Endpoint, String> {
    if let Some(ep) = &options.explicit {
        return Ok(ep.clone());
    }
    discovery::discover(&options.data_dir)
}

fn next_backoff(current: Duration, options: &ClientOptions) -> Duration {
    let doubled = current.saturating_mul(2);
    doubled
        .min(options.reconnect_max)
        .max(options.reconnect_min)
}

async fn run_session(
    endpoint: &discovery::Endpoint,
    options: &ClientOptions,
    events: &flume::Sender<ClientEvent>,
    cmd_rx: &mut flume::Receiver<Command>,
    connected_once: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let url = endpoint.ws_url();

    let (ws, _response) =
        tokio::time::timeout(options.connect_timeout, connect_async(url.as_str()))
            .await
            .map_err(|_| format!("timed out connecting to {url}"))?
            .map_err(|e| format!("connect to {url}: {e}"))?;

    log::info!(target: "myx::soloist", "connected to soloist at {url}");
    let _ = events.send(ClientEvent::Connected {
        endpoint: endpoint.to_string(),
    });

    // On the first successful connect keep any queued
    // commands — they are the startup ones (state,
    // activate, volume). On a reconnect, commands
    // queued while the link was down are stale: drop
    // them so a burst of presses during an outage does
    // not replay once the link returns.
    let first_connect = !connected_once.swap(true, Ordering::SeqCst);
    if !first_connect {
        while cmd_rx.try_recv().is_ok() {}
    }

    let (mut sink, mut stream) = ws.split();
    let mut ping = tokio::time::interval(options.ping_interval);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ping.tick().await; // first tick is immediate

    loop {
        tokio::select! {
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    match ServerEvent::parse(text.as_str()) {
                        Ok(Some(ev)) => {
                            if events.send(ClientEvent::Event(ev)).is_err() {
                                return Ok(()); // app is shutting down
                            }
                        }
                        Ok(None) => {}
                        Err(err) => log::warn!(target: "myx::soloist", "unparseable soloist frame: {err}"),
                    }
                }
                Some(Ok(Message::Ping(payload))) => {
                    // tungstenite queues the pong automatically on
                    // read, but flushing here keeps liveness obvious.
                    if sink.send(Message::Pong(payload)).await.is_err() {
                        return Err("pong failed".into());
                    }
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Close(frame))) => {
                    return Err(format!("server closed the connection: {frame:?}"));
                }
                Some(Ok(_)) => {}
                Some(Err(err)) => return Err(err.to_string()),
                None => return Err("connection lost".into()),
            },
            cmd = cmd_rx.recv_async() => match cmd {
                Ok(cmd) => {
                    let text = serde_json::to_string(&cmd)
                        .map_err(|e| format!("serialize command: {e}"))?;
                    log::debug!(target: "myx::soloist", "send command {}", cmd.command);
                    if sink.send(Message::text(text)).await.is_err() {
                        return Err(format!("send failed: {url}"));
                    }
                }
                Err(_) => return Ok(()), // handle dropped: shutting down
            },
            _ = ping.tick() => {
                if sink
                    .send(Message::Ping(Vec::<u8>::new().into()))
                    .await
                    .is_err()
                {
                    return Err("keepalive failed".into());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_endpoint_wins_over_discovery() {
        let ep = discovery::Endpoint::parse("127.0.0.1:1234").unwrap();
        let opts = ClientOptions {
            explicit: Some(ep.clone()),
            data_dir: std::path::PathBuf::from("/nonexistent"),
            ..Default::default()
        };
        assert_eq!(resolve_endpoint(&opts).unwrap(), ep);
    }

    #[test]
    fn discovery_is_used_when_no_explicit_endpoint() {
        let dir = std::env::temp_dir().join(format!("myx-soloist-client-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ws.addr"), "127.0.0.1").unwrap();
        std::fs::write(dir.join("ws.port"), "9090").unwrap();
        let opts = ClientOptions {
            explicit: None,
            data_dir: dir,
            ..Default::default()
        };
        let ep = resolve_endpoint(&opts).unwrap();
        assert_eq!(ep.port, 9090);
    }

    #[test]
    fn a_closed_command_channel_is_detected() {
        // The session loop ends cleanly when its command
        // handle is dropped: a closed flume channel reports
        // `Disconnected` on the next receive.
        let (tx, rx) = flume::unbounded::<Command>();
        drop(tx);
        assert!(matches!(
            rx.try_recv(),
            Err(flume::TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn backoff_is_bounded_by_the_configured_max() {
        let opts = ClientOptions {
            reconnect_max: Duration::from_secs(10),
            ..Default::default()
        };
        let mut wait = opts.reconnect_min;
        for _ in 0..20 {
            wait = next_backoff(wait, &opts);
            assert!(wait <= opts.reconnect_max, "{wait:?} past the cap");
        }
        assert_eq!(wait, opts.reconnect_max);
    }
}
