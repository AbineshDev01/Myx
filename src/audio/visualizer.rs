//! Low-latency audio observer and waveform analyzer.
//!
//! Captures PCM from the PipeWire output sink without interfering with playback
//! or adding latency, analyzes waveform oscillations and band energy, and emits
//! snapshots for the liquid fluid visualizer.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::watch;

/// Downsampled waveform points sent to the fluid visualizer per snapshot.
pub const WAVEFORM_POINTS: usize = 128;
/// Recent PCM samples retained for waveform downsampling.
pub const PCM_WINDOW_SIZE: usize = 1024;
/// Update cadence in audio frames (~10.6ms at 48kHz).
pub const EMIT_INTERVAL_FRAMES: usize = 512;

/// A point-in-time audio waveform and energy snapshot.
#[derive(Debug, Clone)]
pub struct AudioWaveSnapshot {
    /// Normalized waveform samples in range [-1.0, 1.0].
    pub waveform: Vec<f32>,
    /// Smoothed low-frequency / bass energy in range [0.0, 1.0].
    pub bass_energy: f32,
    /// Smoothed high-frequency / transient energy in range [0.0, 1.0].
    pub treble_energy: f32,
    /// Overall Root-Mean-Square energy in range [0.0, 1.0].
    pub rms: f32,
    /// Instantaneous peak sample value in range [0.0, 1.0].
    pub peak: f32,
    /// Whether audio is actively playing (above noise gate floor).
    pub is_active: bool,
}

impl Default for AudioWaveSnapshot {
    fn default() -> Self {
        Self {
            waveform: vec![0.0; WAVEFORM_POINTS],
            bass_energy: 0.0,
            treble_energy: 0.0,
            rms: 0.0,
            peak: 0.0,
            is_active: false,
        }
    }
}

/// Lifecycle status of the audio observer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisualizerStatus {
    Starting,
    Running,
    Disabled(String),
    Unavailable(String),
}

/// Handle to the background audio visualizer observer.
pub struct VisualizerHandle {
    pub snapshot_rx: watch::Receiver<AudioWaveSnapshot>,
    pub status_rx: watch::Receiver<VisualizerStatus>,
    stop: Arc<AtomicBool>,
}

impl VisualizerHandle {
    pub fn new_disabled(reason: impl Into<String>) -> Self {
        let (snapshot_tx, snapshot_rx) = watch::channel(AudioWaveSnapshot::default());
        let (status_tx, status_rx) =
            watch::channel(VisualizerStatus::Disabled(reason.into()));
        let _ = snapshot_tx;
        let _ = status_tx;
        Self {
            snapshot_rx,
            status_rx,
            stop: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn latest_snapshot(&self) -> AudioWaveSnapshot {
        self.snapshot_rx.borrow().clone()
    }

    pub fn is_active(&self) -> bool {
        self.snapshot_rx.borrow().is_active
    }

    pub fn status(&self) -> VisualizerStatus {
        self.status_rx.borrow().clone()
    }
}

impl Drop for VisualizerHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// PCM analyzer that filters audio and extracts waveform + frequency envelopes.
pub struct WaveAnalyzer {
    pub sensitivity: f32,
    pub rate: u32,
    pub channels: u32,
    buffer: Vec<f32>,
    bass_filter_state: f32,
    prev_sample: f32,
    bass_accum: f32,
    treble_accum: f32,
    energy_accum: f32,
    peak_accum: f32,
    sample_count: usize,
    smoothed_bass: f32,
    smoothed_treble: f32,
    snapshot_tx: watch::Sender<AudioWaveSnapshot>,
    status_tx: watch::Sender<VisualizerStatus>,
}

impl WaveAnalyzer {
    pub fn new(
        sensitivity: f32,
        snapshot_tx: watch::Sender<AudioWaveSnapshot>,
        status_tx: watch::Sender<VisualizerStatus>,
    ) -> Self {
        Self {
            sensitivity: sensitivity.clamp(0.1, 5.0),
            rate: 48_000,
            channels: 2,
            buffer: Vec::with_capacity(PCM_WINDOW_SIZE * 2),
            bass_filter_state: 0.0,
            prev_sample: 0.0,
            bass_accum: 0.0,
            treble_accum: 0.0,
            energy_accum: 0.0,
            peak_accum: 0.0,
            sample_count: 0,
            smoothed_bass: 0.0,
            smoothed_treble: 0.0,
            snapshot_tx,
            status_tx,
        }
    }

    /// Process a block of mono samples.
    pub fn process_mono_samples(&mut self, samples: &[f32]) {
        let alpha = (2.0 * std::f32::consts::PI * 180.0 / (self.rate as f32).max(8000.0)).clamp(0.001, 0.5);

        for &s in samples {
            self.buffer.push(s);
            let abs_s = s.abs();
            if abs_s > self.peak_accum {
                self.peak_accum = abs_s;
            }
            self.energy_accum += s * s;

            // Low-pass filter for bass energy (~180Hz)
            self.bass_filter_state += alpha * (s - self.bass_filter_state);
            self.bass_accum += self.bass_filter_state * self.bass_filter_state;

            // High frequency / transient difference
            let diff = (s - self.prev_sample).abs();
            self.treble_accum += diff;
            self.prev_sample = s;

            self.sample_count += 1;
            if self.sample_count >= EMIT_INTERVAL_FRAMES {
                self.emit_snapshot();
            }
        }

        // Maintain sliding buffer
        if self.buffer.len() > PCM_WINDOW_SIZE * 2 {
            let keep = PCM_WINDOW_SIZE;
            let drop = self.buffer.len() - keep;
            self.buffer.drain(..drop);
        }
    }

    /// Consume interleaved raw bytes (F32LE).
    pub fn consume_bytes(&mut self, bytes: &[u8]) {
        let frame_size = std::mem::size_of::<f32>() * self.channels as usize;
        if frame_size == 0 || bytes.len() < frame_size {
            return;
        }
        let n_frames = bytes.len() / frame_size;
        let words: &[f32] = unsafe {
            std::slice::from_raw_parts(
                bytes.as_ptr().cast::<f32>(),
                bytes.len() / std::mem::size_of::<f32>(),
            )
        };

        let ch = self.channels as usize;
        let mut mono = Vec::with_capacity(n_frames);
        for f in 0..n_frames {
            let mut sum = 0.0f32;
            for c in 0..ch {
                if let Some(&val) = words.get(f * ch + c) {
                    sum += val;
                }
            }
            mono.push(sum / ch as f32);
        }

        self.process_mono_samples(&mono);
    }

    fn emit_snapshot(&mut self) {
        let n = self.sample_count.max(1) as f32;
        let rms = (self.energy_accum / n).sqrt();
        let bass_rms = (self.bass_accum / n).sqrt();
        let treble_avg = self.treble_accum / n;
        let peak = self.peak_accum;

        // Reset accumulators
        self.sample_count = 0;
        self.energy_accum = 0.0;
        self.bass_accum = 0.0;
        self.treble_accum = 0.0;
        self.peak_accum = 0.0;

        // Noise gate floor
        let is_active = peak > 0.0035 && rms > 0.0015;

        // Map bass & treble to 0..1 range with sensitivity
        let target_bass = (bass_rms * 4.5 * self.sensitivity).clamp(0.0, 1.0);
        let target_treble = (treble_avg * 6.0 * self.sensitivity).clamp(0.0, 1.0);

        // Smooth with attack/release dynamics
        let bass_attack = 0.65;
        let bass_release = 0.18;
        self.smoothed_bass = if target_bass > self.smoothed_bass {
            self.smoothed_bass + (target_bass - self.smoothed_bass) * bass_attack
        } else {
            self.smoothed_bass + (target_bass - self.smoothed_bass) * bass_release
        };

        let treble_attack = 0.70;
        let treble_release = 0.25;
        self.smoothed_treble = if target_treble > self.smoothed_treble {
            self.smoothed_treble + (target_treble - self.smoothed_treble) * treble_attack
        } else {
            self.smoothed_treble + (target_treble - self.smoothed_treble) * treble_release
        };

        // Extract downsampled waveform window
        let mut waveform = vec![0.0f32; WAVEFORM_POINTS];
        if is_active && !self.buffer.is_empty() {
            let window_len = self.buffer.len().min(PCM_WINDOW_SIZE);
            let start = self.buffer.len() - window_len;
            let slice = &self.buffer[start..];

            let chunk_size = (slice.len() as f32 / WAVEFORM_POINTS as f32).max(1.0);
            for (i, slot) in waveform.iter_mut().enumerate() {
                let c_start = (i as f32 * chunk_size) as usize;
                let c_end = (((i + 1) as f32 * chunk_size) as usize).min(slice.len()).max(c_start + 1);
                let chunk = &slice[c_start..c_end];
                let avg = chunk.iter().sum::<f32>() / chunk.len() as f32;
                *slot = (avg * self.sensitivity).clamp(-1.0, 1.0);
            }
        }

        let snapshot = AudioWaveSnapshot {
            waveform,
            bass_energy: if is_active { self.smoothed_bass } else { 0.0 },
            treble_energy: if is_active { self.smoothed_treble } else { 0.0 },
            rms: if is_active { (rms * self.sensitivity).clamp(0.0, 1.0) } else { 0.0 },
            peak: if is_active { (peak * self.sensitivity).clamp(0.0, 1.0) } else { 0.0 },
            is_active,
        };

        let _ = self.snapshot_tx.send_replace(snapshot);
    }
}

/// Spawn the audio observer.
pub fn spawn(enabled: bool, sensitivity: f32, target: String) -> VisualizerHandle {
    let (snapshot_tx, snapshot_rx) = watch::channel(AudioWaveSnapshot::default());
    let (status_tx, status_rx) = watch::channel(VisualizerStatus::Starting);
    let stop = Arc::new(AtomicBool::new(false));

    if !enabled {
        let _ = status_tx.send_replace(VisualizerStatus::Disabled("disabled in config".into()));
        return VisualizerHandle {
            snapshot_rx,
            status_rx,
            stop,
        };
    }

    #[cfg(all(target_os = "linux", feature = "streaming"))]
    {
        let stop_clone = stop.clone();
        let target_clone = target;
        let thread_res = std::thread::Builder::new()
            .name("myx-visualizer".into())
            .spawn(move || {
                run_pipewire_observer(sensitivity, target_clone, snapshot_tx, status_tx, stop_clone);
            });

        if let Err(e) = thread_res {
            return VisualizerHandle::new_disabled(format!("thread spawn failed: {e}"));
        }
    }

    #[cfg(not(all(target_os = "linux", feature = "streaming")))]
    {
        let _ = target;
        let _ = sensitivity;
        let _ = status_tx.send_replace(VisualizerStatus::Unavailable(
            "pipewire observer requires linux with streaming feature".into(),
        ));
    }

    VisualizerHandle {
        snapshot_rx,
        status_rx,
        stop,
    }
}

#[cfg(all(target_os = "linux", feature = "streaming"))]
fn run_pipewire_observer(
    sensitivity: f32,
    target: String,
    snapshot_tx: watch::Sender<AudioWaveSnapshot>,
    status_tx: watch::Sender<VisualizerStatus>,
    stop: Arc<AtomicBool>,
) {
    if let Err(err) = run_pipewire_inner(sensitivity, target, &snapshot_tx, &status_tx, &stop) {
        log::debug!("visualizer pipewire observer: {err}");
        let _ = status_tx.send_replace(VisualizerStatus::Unavailable(err.to_string()));
    }
}

#[cfg(all(target_os = "linux", feature = "streaming"))]
fn run_pipewire_inner(
    sensitivity: f32,
    target: String,
    snapshot_tx: &watch::Sender<AudioWaveSnapshot>,
    status_tx: &watch::Sender<VisualizerStatus>,
    _stop: &Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use pipewire as pw;
    use pw::properties::properties;
    use pw::spa;
    use pw::spa::param::format::{MediaSubtype, MediaType};
    use pw::spa::param::format_utils;

    pw::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;

    let mut props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::STREAM_CAPTURE_SINK => "true",
    };
    if !target.is_empty() {
        props.insert("target.object", target.clone());
    }

    let stream = pw::stream::StreamRc::new(core, "myx-fluid-visualizer", props)?;

    let analyzer = WaveAnalyzer::new(sensitivity, snapshot_tx.clone(), status_tx.clone());
    let _listener = stream
        .add_local_listener_with_user_data(analyzer)
        .param_changed(|_, user_data, id, param| {
            if id != pw::spa::param::ParamType::Format.as_raw() {
                return;
            }
            let Some(param) = param else {
                return;
            };
            let (media_type, media_subtype) = match format_utils::parse_format(param) {
                Ok(v) => v,
                Err(_) => return,
            };
            if media_type != MediaType::Audio || media_subtype != MediaSubtype::Raw {
                return;
            }
            let mut info = spa::param::audio::AudioInfoRaw::default();
            if info.parse(param).is_err() {
                return;
            }
            let (rate, channels) = (info.rate(), info.channels().max(1));
            user_data.rate = rate;
            user_data.channels = channels;
            let _ = user_data.status_tx.send_replace(VisualizerStatus::Running);
        })
        .process(|stream, user_data| match stream.dequeue_buffer() {
            None => {}
            Some(mut buffer) => {
                let datas = buffer.datas_mut();
                if let Some(data) = datas.first_mut() {
                    let chunk = data.chunk();
                    let size = chunk.size() as usize;
                    if let Some(bytes) = data.data() {
                        let bytes = &bytes[..size.min(bytes.len())];
                        user_data.consume_bytes(bytes);
                    }
                }
            }
        })
        .register()?;

    // Request raw F32 audio format
    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    let obj = pw::spa::pod::Object {
        type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: pw::spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(obj),
    )?
    .0
    .into_inner();

    let pod = pw::spa::pod::Pod::from_bytes(&values).ok_or("invalid format pod")?;
    let mut params = [pod];
    stream.connect(
        spa::utils::Direction::Input,
        None,
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut params,
    )?;

    let _ = status_tx.send_replace(VisualizerStatus::Running);
    mainloop.run();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wave_analyzer_detects_silence_and_active_signal() {
        let (snap_tx, snap_rx) = watch::channel(AudioWaveSnapshot::default());
        let (stat_tx, _stat_rx) = watch::channel(VisualizerStatus::Starting);
        let mut analyzer = WaveAnalyzer::new(1.0, snap_tx, stat_tx);

        // Feed silence
        let silence = vec![0.0f32; EMIT_INTERVAL_FRAMES];
        analyzer.process_mono_samples(&silence);
        let s0 = snap_rx.borrow().clone();
        assert!(!s0.is_active);
        assert_eq!(s0.bass_energy, 0.0);
        assert_eq!(s0.rms, 0.0);

        // Feed 100Hz sine wave (bass excitation)
        let mut sine = Vec::with_capacity(EMIT_INTERVAL_FRAMES);
        for i in 0..EMIT_INTERVAL_FRAMES {
            let t = i as f32 / 48000.0;
            sine.push((2.0 * std::f32::consts::PI * 100.0 * t).sin() * 0.7);
        }
        analyzer.process_mono_samples(&sine);
        let s1 = snap_rx.borrow().clone();
        assert!(s1.is_active);
        assert!(s1.rms > 0.2);
        assert!(s1.bass_energy > 0.1);
        assert_eq!(s1.waveform.len(), WAVEFORM_POINTS);
    }
}
