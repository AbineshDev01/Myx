//! Route Soloist's audio through myx's own sink chain.
//!
//! Spotify Soloist decodes and plays its own audio, so myx's
//! equalizer never sees a single sample of it. This module puts
//! myx back in the path: it creates a virtual PipeWire/PulseAudio
//! sink, points Soloist at it, and captures that sink's monitor —
//! pumping the captured PCM through the `EqualizerSink` + [`RodioSink`]
//! chain. The equalizer therefore applies exactly as before.
//!
//! It depends on `pactl` and `parec` (from `pulseaudio-utils` or
//! `pipewire-pulse`). When they are missing [`AudioTap::start`]
//! fails with a readable reason, and the caller lets Soloist play
//! directly.
//!
//! The pipeline is deliberately fail-soft: a missing tool, a
//! refused sink, or an output backend that will not start all
//! surface as an `Err` so the caller can keep Soloist audible.

use std::io::Read;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::audio::backend::{AudioPacket, RodioSink, Sink};
use crate::audio::equalizer::{EqualizerControl, EqualizerSink};
use crate::audio::{VisBands, VisualizationSink};

/// Capture rate. `parec` is told to deliver 44.1 kHz stereo S16LE,
/// which is the rate the whole chain (equalizer, rodio source)
/// is built around, so the captured packets need no resampling.
const RATE: u32 = 44_100;
const CHANNELS: u32 = 2;
/// Bytes per read — a whole number of 4-byte stereo S16 frames (256 frames ≈ 5.8 ms).
const READ_BYTES: usize = 4 * 256;
/// How long the output backend may take to come up before we give
/// up on it.
const START_TIMEOUT: Duration = Duration::from_secs(5);
/// Captured silence for this long is almost certainly a mis-routed
/// sink.
const SILENCE_WARN: Duration = Duration::from_secs(8);
/// Below this, a sample is treated as silence for the warning above.
const SILENCE_FLOOR: f64 = 1e-4;

/// A virtual sink plus the capture that feeds myx's sink chain.
pub struct AudioTap {
    sink: String,
    module: Option<u32>,
    capture: Option<Child>,
    stop: Arc<AtomicBool>,
    clear: Arc<AtomicBool>,
    pump: Option<JoinHandle<()>>,
}

/// Myx's own sink chain — the equalizer curve — as an owned handle.
///
/// The engine hands one out so a second output can be built on
/// another thread: [`AudioTap::start`] would otherwise have to
/// borrow the engine, which is exactly what a worker thread cannot
/// do.
/// Myx's own sink chain — the visualizer bands and the equalizer
/// curve — as an owned handle.
///
/// The engine hands one out so a second output can be built on
/// another thread: [`AudioTap::start`] would otherwise have to
/// borrow the engine, which is exactly what a worker thread cannot
/// do. Both halves are shared with myx's own player, so the
/// spectrum drawn and the equalizer applied are the same ones it
/// uses.
#[derive(Clone)]
pub struct SinkChain {
    bands: Arc<Mutex<VisBands>>,
    equalizer: EqualizerControl,
}

impl SinkChain {
    pub(crate) fn new(bands: Arc<Mutex<VisBands>>, equalizer: EqualizerControl) -> Self {
        Self { bands, equalizer }
    }

    /// Take the chain apart, ready to be moved onto the pump thread.
    fn into_parts(self) -> (Arc<Mutex<VisBands>>, EqualizerControl) {
        (self.bands, self.equalizer)
    }
}

impl AudioTap {
    /// Create the virtual sink and start capturing it.
    ///
    /// Soloist must be pointed at [`sink_name`](Self::sink_name)
    /// (`--pipewire-device` and/or `PULSE_SINK`) or its audio will
    /// never reach myx.
    ///
    /// This waits for the output backend to actually come up: until
    /// myx can play what it captures, pointing Soloist at the sink
    /// would mean silence rather than a fallback. That wait can take
    /// a moment, so callers must not run it on the UI thread.
    pub fn start(chain: SinkChain) -> Result<Self, String> {
        // Scoped to the process so a module left behind by a crash
        // can never collide with this run's sink.
        let sink = format!("myx_soloist_{}", std::process::id());
        let module = load_null_sink(&sink)?;

        let mut capture = match spawn_capture(&sink) {
            Ok(child) => child,
            Err(e) => {
                unload_module(module);
                return Err(e);
            }
        };
        let Some(stdout) = capture.stdout.take() else {
            let _ = capture.kill();
            unload_module(module);
            return Err("capture process has no stdout".to_string());
        };

        let (bands, equalizer) = chain.into_parts();
        let stop = Arc::new(AtomicBool::new(false));
        let clear = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel();
        let pump = match thread::Builder::new()
            .name("myx-soloist-tap".to_string())
            .spawn({
                let stop = Arc::clone(&stop);
                let clear = Arc::clone(&clear);
                move || pump(stdout, bands, equalizer, stop, clear, ready_tx)
            }) {
            Ok(handle) => handle,
            Err(e) => {
                let _ = capture.kill();
                unload_module(module);
                return Err(format!("spawn capture thread: {e}"));
            }
        };

        // The output backend is built on the pump thread (opening an
        // audio device must never stall the UI), so its failure has
        // to travel back before we claim success — the caller falls
        // back to direct playback on any error.
        match ready_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                sink,
                module: Some(module),
                capture: Some(capture),
                stop,
                clear,
                pump: Some(pump),
            }),
            Ok(Err(e)) => {
                let _ = capture.kill();
                unload_module(module);
                Err(e)
            }
            Err(_) => {
                let _ = capture.kill();
                unload_module(module);
                Err("audio output did not start in time".to_string())
            }
        }
    }

    /// The virtual sink's name — where Soloist must play.
    pub fn sink_name(&self) -> &str {
        &self.sink
    }

    /// Clear buffered audio immediately so transport actions respond in real time.
    pub fn clear(&self) {
        self.clear.store(true, Ordering::Relaxed);
    }
}

impl Drop for AudioTap {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Killing the capture ends the pump's read and lets the
        // thread clean up.
        if let Some(mut child) = self.capture.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(pump) = self.pump.take() {
            let _ = pump.join();
        }
        if let Some(module) = self.module.take() {
            unload_module(module);
        }
    }
}

/// Build myx's sink chain here and stream the captured PCM through
/// it.
///
/// The chain is built on this thread because it owns the audio
/// device; `ready` reports success or the reason the output could
/// not start.
fn pump(
    mut stdout: ChildStdout,
    bands: Arc<Mutex<VisBands>>,
    equalizer: EqualizerControl,
    stop: Arc<AtomicBool>,
    clear: Arc<AtomicBool>,
    ready: mpsc::Sender<Result<(), String>>,
) {
    let real = match RodioSink::new() {
        Ok(real) => real,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let visualized = VisualizationSink::new(Box::new(real), Arc::clone(&bands), RATE as f32);
    let mut sink: Box<dyn Sink> = Box::new(EqualizerSink::new(Box::new(visualized), equalizer));
    if let Err(e) = sink.start() {
        let _ = ready.send(Err(format!("start audio output: {e}")));
        return;
    }
    let _ = ready.send(Ok(()));

    let mut bytes = vec![0u8; READ_BYTES];
    let mut carry: Vec<u8> = Vec::new();
    let started = Instant::now();
    let mut heard_audio = false;
    let mut warned = false;
    let mut active = false;

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if clear.swap(false, Ordering::Relaxed) {
            carry.clear();
            let _ = sink.clear();
        }
        let n = match stdout.read(&mut bytes) {
            Ok(0) | Err(_) => break, // parec exited or the pipe broke
            Ok(n) => n,
        };
        carry.extend_from_slice(&bytes[..n]);
        // Hold any trailing partial frame for the next read.
        let usable = carry.len() - carry.len() % 4;
        if usable == 0 {
            continue;
        }
        let samples = pcm_to_samples(&carry[..usable]);
        carry.drain(..usable);

        if !heard_audio && samples.iter().any(|s| s.abs() > SILENCE_FLOOR) {
            heard_audio = true;
        }
        if !warned && !heard_audio && started.elapsed() >= SILENCE_WARN {
            warned = true;
            crate::liblog::liblog(
                "audio tap: no audio captured yet — is Soloist routed to the virtual sink?",
            );
        }

        if !active {
            if let Ok(mut b) = bands.lock() {
                b.is_active = true;
            }
            active = true;
        }

        if sink.write(AudioPacket::Samples(samples)).is_err() {
            break;
        }
    }

    let _ = sink.stop();
    if active {
        if let Ok(mut b) = bands.lock() {
            b.is_active = false;
            b.values.fill(0.0);
            b.peak_envelope = 1e-6;
        }
    }
}

/// Interleaved little-endian S16 -> normalized `f64` samples.
fn pcm_to_samples(bytes: &[u8]) -> Vec<f64> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b) as f64 / 32_768.0)
        .collect()
}

/// Create the null sink; the returned id removes it again.
fn load_null_sink(name: &str) -> Result<u32, String> {
    let out = Command::new("pactl")
        .args([
            "load-module",
            "module-null-sink",
            &format!("sink_name={name}"),
            &format!("sink_properties=device.description={name}"),
        ])
        .output()
        .map_err(|e| missing("pactl", e))?;
    if !out.status.success() {
        return Err(format!(
            "could not create the audio sink: {}",
            first_line(&out.stderr)
        ));
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u32>()
        .map_err(|_| "the audio sink was not created (no module id)".to_string())
}

fn unload_module(module: u32) {
    let _ = Command::new("pactl")
        .args(["unload-module", &module.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Capture the null sink's monitor as raw 44.1 kHz stereo S16LE.
fn spawn_capture(sink: &str) -> Result<Child, String> {
    Command::new("parec")
        .args([
            &format!("--device={sink}.monitor"),
            "--format=s16le",
            &format!("--rate={RATE}"),
            &format!("--channels={CHANNELS}"),
            "--client-name=myx-soloist-tap",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| missing("parec", e))
}

fn missing(tool: &str, e: std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound {
        format!("{tool} not found — install pulseaudio-utils or pipewire-pulse")
    } else {
        format!("could not run {tool}: {e}")
    }
}

fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_converts_interleaved_s16_to_normalized_floats() {
        // Full-scale negative, silence, full-scale positive, half.
        let bytes = [0x00, 0x80, 0x00, 0x00, 0xFF, 0x7F, 0x00, 0x40];
        let samples = pcm_to_samples(&bytes);
        assert_eq!(samples.len(), 4);
        assert!((samples[0] + 1.0).abs() < 1e-9); // -32768
        assert_eq!(samples[1], 0.0);
        assert!((samples[2] - 32767.0 / 32768.0).abs() < 1e-9);
        assert!((samples[3] - 0.5).abs() < 1e-9); // 16384
    }

    #[test]
    fn pcm_ignores_a_trailing_partial_sample() {
        // Three bytes: one whole S16 frame plus a stray byte, which
        // the pump holds back rather than decoding.
        assert_eq!(pcm_to_samples(&[0x00, 0x00, 0x7F]).len(), 1);
    }

    #[test]
    fn first_line_trims_and_takes_only_the_first() {
        assert_eq!(first_line(b"  42 \nwarning: noise\n"), "42");
        assert_eq!(first_line(b""), "");
    }
}
