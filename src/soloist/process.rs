//! Soloist process lifecycle.
//!
//! Myx needs a running Soloist with its WebSocket API
//! (`--ws`) before it can play anything. This module makes
//! sure one exists and is reachable, without ever starting
//! a duplicate:
//!
//! * A Soloist that is already running **and** exposes its
//!   WebSocket API is reused as-is and left running when
//!   Myx exits.
//! * Otherwise Myx starts one itself, pointed at the
//!   loopback tap's sink so the equalizer keeps seeing
//!   the audio, and owns it until shutdown.
//!
//! Soloist resolves its own data directory; Myx reads the
//! same directory to discover the WebSocket endpoint
//! (`ws.addr` / `ws.port`) and the pid file.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use libc::{SIGKILL, SIGTERM};

use super::discovery;
use crate::audio::tap::AudioTap;
use crate::audio::EqualizerControl;

/// How long to wait for Soloist to expose its WebSocket API.
const READINESS_TIMEOUT: Duration = Duration::from_secs(20);
/// Poll cadence while waiting for readiness.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

pub struct StartOptions {
    pub api_key: Option<String>,
    pub device_name: String,
    pub executable: String,
    /// Route Soloist's audio through myx's loopback tap so
    /// the equalizer keeps working.
    pub route_audio: bool,
    /// Override Soloist's data directory (sets
    /// `STATE_DIRECTORY` for the child).
    pub data_dir: Option<PathBuf>,
    pub equalizer: EqualizerControl,
}

enum Kind {
    /// A Soloist someone else started; Myx only talks to it.
    External,
    /// A Soloist Myx started and therefore owns.
    Launched { child: Child },
}

pub struct SoloistProcess {
    kind: Kind,
    data_dir: PathBuf,
    /// Kept alive so the loopback tap keeps running for as
    /// long as Myx owns the Soloist instance.
    _tap: Option<AudioTap>,
}

impl SoloistProcess {
    /// Ensure a usable Soloist is running and return a handle
    /// to it. Never blocks the caller indefinitely: readiness
    /// is bounded by [`READINESS_TIMEOUT`].
    pub async fn start(opts: StartOptions) -> Result<Self> {
        let data_dir = opts
            .data_dir
            .clone()
            .unwrap_or_else(discovery::default_data_dir);

        // Reuse a Soloist that is already running and already
        // exposes its WebSocket API.
        if let Some(endpoint) = reuse_running(&data_dir) {
            log::info!(target: "myx::soloist", "reusing running Soloist at {endpoint}");
            return Ok(Self {
                kind: Kind::External,
                data_dir,
                _tap: None,
            });
        }

        // A Soloist is running but without its WebSocket API:
        // Myx cannot control it, and Soloist refuses to share
        // its data directory, so there is nothing to do but
        // tell the user how to fix it.
        if discovery::daemon_running(&data_dir) {
            bail!(
                "Soloist is running without its WebSocket API. \
                 Restart it with `soloist --ws`, stop it, or set \
                 `MYX_SOLOIST_WS` / `[soloist] ws` to its endpoint."
            );
        }

        let api_key = opts.api_key;
        let device_name = opts.device_name;

        // The loopback sink, when audio routing is enabled.
        let tap = if opts.route_audio {
            let chain = crate::audio::SinkChain::new(opts.equalizer.clone());
            match AudioTap::start(chain) {
                Ok(tap) => Some(tap),
                Err(err) => {
                    // No compatible audio server: Soloist will
                    // still play to its own default output, but
                    // the equalizer cannot see the audio.
                    // Keep playback working and warn.
                    log::warn!(
                        target: "myx::soloist",
                        "loopback tap unavailable ({err}); equalizer is inactive"
                    );
                    None
                }
            }
        } else {
            None
        };
        let sink_name = tap.as_ref().map(|t| t.sink_name().to_string());

        // Prefer the documented `--pipewire-device` option;
        // fall back to the `PULSE_SINK` environment variable
        // for builds (or setups) that only speak PulseAudio.
        let launched = match spawn_and_wait(
            &opts.executable,
            &api_key,
            &device_name,
            &sink_name,
            true,
            &data_dir,
        )
        .await
        {
            Ok(child) => child,
            Err(SpawnOutcome::OptionUnsupported) => spawn_and_wait(
                &opts.executable,
                &api_key,
                &device_name,
                &sink_name,
                false,
                &data_dir,
            )
            .await
            .map_err(|e| anyhow::anyhow!("starting Soloist with PULSE_SINK: {e:?}"))?,
            Err(SpawnOutcome::Failed(err)) => {
                bail!("starting Soloist: {err}");
            }
        };

        Ok(Self {
            kind: Kind::Launched { child: launched },
            data_dir,
            _tap: tap,
        })
    }

    /// The data directory Soloist writes its runtime files to.
    /// The WebSocket client re-discovers the endpoint here on
    /// every reconnect, so Soloist restarting is recovered
    /// automatically.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Whether Myx started this Soloist (and therefore owns
    /// its shutdown).
    pub fn is_external(&self) -> bool {
        matches!(self.kind, Kind::External)
    }

    /// Clear the loopback tap's audio buffer immediately if active.
    pub fn clear_tap(&self) {
        if let Some(tap) = &self._tap {
            tap.clear();
        }
    }

    /// Shut down Soloist. Only the instance Myx started is
    /// terminated; an independently started Soloist is left
    /// running. Tries a graceful `SIGTERM` first, then a
    /// forced `SIGKILL`.
    pub fn shutdown(&mut self) {
        match &mut self.kind {
            Kind::External => {
                log::info!(
                    target: "myx::soloist",
                    "leaving independently started Soloist running"
                );
            }
            Kind::Launched { child } => {
                shutdown_child(child);
            }
        }
    }
}

/// Outcome of a Soloist spawn attempt.
#[derive(Debug)]
enum SpawnOutcome {
    /// `--pipewire-device` is not supported by this build.
    OptionUnsupported,
    /// Soloist failed to start or never became ready.
    Failed(String),
}

/// Spawn Soloist and wait until it exposes its WebSocket API
/// (or fails). `use_pipewire` selects `--pipewire-device`;
/// otherwise the sink is selected via `PULSE_SINK`.
async fn spawn_and_wait(
    executable: &str,
    api_key: &Option<String>,
    device_name: &str,
    sink_name: &Option<String>,
    use_pipewire: bool,
    data_dir: &Path,
) -> Result<Child, SpawnOutcome> {
    let mut command = Command::new(executable);
    command
        .arg("--device-name")
        .arg(device_name)
        .arg("--ws")
        .arg("127.0.0.1:0");
    if let Some(key) = api_key {
        command.arg("--api-key").arg(key);
    }
    if use_pipewire {
        if let Some(sink) = sink_name {
            command.arg("--pipewire-device").arg(sink);
        }
    }
    // `PULSE_SINK` is harmless when unused and is the only
    // mechanism on PulseAudio / PipeWire-Pulse setups.
    if let Some(sink) = sink_name {
        command.env("PULSE_SINK", sink);
    }
    // Pin the child to the same data directory Myx reads
    // its runtime files from, so discovery and the child
    // always agree.
    command.env("STATE_DIRECTORY", data_dir);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    // Kill Soloist if myx dies unexpectedly so we never leak
    // an owned process.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, SIGKILL as usize, 0, 0, 0);
                Ok(())
            });
        }
    }

    let mut child = command
        .spawn()
        .map_err(|e| SpawnOutcome::Failed(format!("spawn `{executable}`: {e}")))?;

    // Drain stdout continuously to prevent pipe buffer starvation and keep
    // child process log messages from leaking into the terminal TUI.
    if let Some(stdout) = child.stdout.take() {
        thread::spawn(move || drain_stdout(stdout));
    }

    // Drain stderr continuously (so the pipe never fills and
    // blocks Soloist) while retaining the tail for error
    // reporting and option-detection.
    let captured = Arc::new(Mutex::new(String::new()));
    if let Some(stderr) = child.stderr.take() {
        let captured = Arc::clone(&captured);
        thread::spawn(move || drain_stderr(stderr, captured));
    }

    let started = Instant::now();
    loop {
        // Soloist exited on its own. Distinguish an
        // unsupported option (retry with PULSE_SINK) from a
        // real startup failure.
        if let Ok(Some(status)) = child.try_wait() {
            let text = captured.lock().unwrap().clone();
            if use_pipewire && mentions_option_error(&text) {
                return Err(SpawnOutcome::OptionUnsupported);
            }
            let reason = first_line(&text);
            return Err(SpawnOutcome::Failed(format!(
                "exited during startup (code {}): {reason}",
                status.code().unwrap_or(-1)
            )));
        }

        // The WebSocket runtime files appear once Soloist is
        // ready to accept connections.
        if discovery::discover(data_dir).is_ok() {
            return Ok(child);
        }

        if started.elapsed() >= READINESS_TIMEOUT {
            return Err(SpawnOutcome::Failed(
                "timed out waiting for the Soloist WebSocket API".into(),
            ));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Continuously drain the child process's stdout and route it to
/// the log file if logging is enabled, preventing pipe starvation and
/// preventing any log output from leaking into the terminal TUI.
fn drain_stdout(stdout: std::process::ChildStdout) {
    use std::io::{BufRead, BufReader};
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        match line {
            Ok(line) => {
                crate::liblog::liblog(format!("soloist: {line}"));
            }
            Err(_) => break,
        }
    }
}

/// Copy a child's stderr into a bounded buffer, keeping the
/// last few kilobytes.
fn drain_stderr(mut stderr: std::process::ChildStderr, captured: Arc<Mutex<String>>) {
    use std::io::Read;
    let mut buf = [0u8; 4096];
    loop {
        match stderr.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&buf[..n]);
                for line in chunk.lines() {
                    crate::liblog::liblog(format!("soloist err: {line}"));
                }
                let mut text = captured.lock().unwrap();
                if text.len() > 8192 {
                    let drop = text.len() - 8192;
                    text.drain(..drop);
                }
                text.push_str(&chunk);
            }
            Err(_) => break,
        }
    }
}

/// Whether a startup error looks like an unrecognised
/// command-line option (clap-style argument errors).
fn mentions_option_error(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("unexpected argument")
        || lower.contains("unrecognized")
        || lower.contains("unknown option")
        || lower.contains("unexpected parameter")
}

/// If a Soloist is already running and exposes its WebSocket
/// API, return its endpoint so Myx can reuse it.
fn reuse_running(data_dir: &Path) -> Option<discovery::Endpoint> {
    discovery::discover(data_dir).ok()
}

fn first_line(bytes: &str) -> String {
    bytes.lines().next().unwrap_or("").trim().to_string()
}

/// Gracefully terminate a child we own: `SIGTERM`, wait a
/// moment, then `SIGKILL` if it is still alive.
fn shutdown_child(child: &mut Child) {
    let id = child.id();
    // Ask nicely first.
    let _ = unsafe { libc::kill(id as i32, SIGTERM) };
    // Give it a short grace period to clean up (Soloist
    // removes its runtime files on exit).
    for _ in 0..20 {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(_) => return,
        }
    }
    log::warn!(
        target: "myx::soloist",
        "Soloist (pid {id}) did not exit on SIGTERM; sending SIGKILL"
    );
    let _ = unsafe { libc::kill(id as i32, SIGKILL) };
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_errors_are_detected() {
        assert!(mentions_option_error(
            "error: unexpected argument '--pipewire-device'"
        ));
        assert!(mentions_option_error("unrecognized option"));
        assert!(mentions_option_error("Unknown option: --foo"));
        assert!(!mentions_option_error("connection refused"));
        assert!(!mentions_option_error(""));
    }

    #[test]
    fn first_line_trims_multi_line_output() {
        assert_eq!(first_line("boom\nat line 3\n"), "boom");
        assert_eq!(first_line(""), "");
    }

    #[test]
    fn stderr_drain_keeps_the_tail() {
        let captured = Arc::new(Mutex::new(String::new()));
        // Simulate more than the retained window so the
        // bounded trim is exercised.
        {
            let mut text = captured.lock().unwrap();
            for _ in 0..20 {
                text.push_str(&"x".repeat(1024));
            }
            let drop = text.len() - 8192;
            text.drain(..drop);
        }
        assert!(captured.lock().unwrap().len() <= 8192);
    }

    #[test]
    fn reuse_returns_none_when_no_daemon() {
        let dir = std::env::temp_dir().join(format!("myx-soloist-noreuse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(reuse_running(&dir).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
