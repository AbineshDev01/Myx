//! Audio visualizer sink and spectrum analyzer.
//!
//! Provides the FFT spectrum analyzer that powers the spectrum bars under
//! the album art. Supports two paths to ensure full Soloist playback coverage:
//! 1. `run_pipewire_observer`: Captures the audio sink via PipeWire with zero
//!    latency when Soloist plays directly to the system audio output.
//! 2. `VisualizationSink`: A tee'd sink used by `AudioTap` when loopback
//!    routing (`route_audio: true`) is enabled.
//!
//! Both paths write frequency bands into a shared `Arc<Mutex<VisBands>>`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rustfft::{num_complex::Complex, FftPlanner};
use tokio::sync::watch;

use crate::audio::backend::{AudioPacket, Sink, SinkResult};

pub const FFT_SIZE: usize = 1024;
/// New samples consumed per FFT frame (overlap = FFT_SIZE - HOP_SIZE).
pub const HOP_SIZE: usize = 128;
pub const NUM_BANDS: usize = 128;

/// Per-frame decay for individual bands — snappy but not jittery.
pub const DECAY_FACTOR: f32 = 0.985;
/// Slower decay for the normalization envelope so quiet passages read quiet.
pub const DECAY_FACTOR_PEAK: f32 = 0.9985;

/// Shared frequency-band state written by the audio sink/observer, read by the renderer.
#[derive(Clone, Debug)]
pub struct VisBands {
    pub values: [f32; NUM_BANDS],
    pub updated_at: Instant,
    pub peak_envelope: f32,
    pub is_active: bool,
}

impl VisBands {
    pub fn new() -> Self {
        Self {
            values: [0.0; NUM_BANDS],
            updated_at: Instant::now(),
            peak_envelope: 1e-6,
            is_active: false,
        }
    }

    pub fn shared() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self::new()))
    }
}

impl Default for VisBands {
    fn default() -> Self {
        Self::new()
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
    pub status_rx: watch::Receiver<VisualizerStatus>,
    stop: Arc<AtomicBool>,
}

impl VisualizerHandle {
    pub fn new_disabled(reason: impl Into<String>) -> Self {
        let (_, status_rx) =
            watch::channel(VisualizerStatus::Disabled(reason.into()));
        Self {
            status_rx,
            stop: Arc::new(AtomicBool::new(true)),
        }
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

/// Core FFT and logarithmic band analyzer that writes into `VisBands`.
pub struct FftProcessor {
    sample_buf: VecDeque<f32>,
    bands: Arc<Mutex<VisBands>>,
    fft: Arc<dyn rustfft::Fft<f32>>,
    hann_window: Vec<f32>,
    fft_buf: Vec<Complex<f32>>,
    magnitudes: Vec<f32>,
    pub sample_rate: f32,
    band_ranges: Vec<(usize, usize)>,
    new_bands: [f32; NUM_BANDS],
    smooth_scratch: [f32; NUM_BANDS],
    pub sensitivity: f32,
}

impl FftProcessor {
    pub fn new(bands: Arc<Mutex<VisBands>>, sample_rate: f32, sensitivity: f32) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let hann_window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (FFT_SIZE - 1) as f32).cos())
            })
            .collect();
        let band_ranges = precompute_band_ranges(FFT_SIZE / 2, NUM_BANDS);
        Self {
            sample_buf: VecDeque::with_capacity(FFT_SIZE * 2),
            bands,
            fft,
            hann_window,
            fft_buf: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            magnitudes: vec![0.0; FFT_SIZE / 2],
            sample_rate: sample_rate.max(8000.0),
            band_ranges,
            new_bands: [0.0; NUM_BANDS],
            smooth_scratch: [0.0; NUM_BANDS],
            sensitivity: sensitivity.clamp(0.1, 5.0),
        }
    }

    pub fn process_mono_samples(&mut self, samples: &[f32]) {
        self.sample_buf.extend(samples.iter().copied());

        while self.sample_buf.len() >= FFT_SIZE {
            {
                let (front, back) = self.sample_buf.as_slices();
                if front.len() >= FFT_SIZE {
                    for (dst, (&s, &w)) in self
                        .fft_buf
                        .iter_mut()
                        .zip(front.iter().zip(self.hann_window.iter()))
                    {
                        *dst = Complex::new(s * w * self.sensitivity, 0.0);
                    }
                } else {
                    let split = front.len();
                    for (dst, (&s, &w)) in self.fft_buf[..split]
                        .iter_mut()
                        .zip(front.iter().zip(self.hann_window[..split].iter()))
                    {
                        *dst = Complex::new(s * w * self.sensitivity, 0.0);
                    }
                    let remaining = FFT_SIZE - split;
                    for (dst, (&s, &w)) in self.fft_buf[split..].iter_mut().zip(
                        back[..remaining]
                            .iter()
                            .zip(self.hann_window[split..].iter()),
                    ) {
                        *dst = Complex::new(s * w * self.sensitivity, 0.0);
                    }
                }
            }

            self.fft.process(&mut self.fft_buf);

            for (mag, c) in self.magnitudes.iter_mut().zip(self.fft_buf.iter()) {
                *mag = c.norm();
            }

            fill_log_bands(&self.magnitudes, &self.band_ranges, &mut self.new_bands);
            smooth_bands(&mut self.new_bands, &mut self.smooth_scratch);

            let frame_peak = self.new_bands.iter().copied().fold(0.0_f32, f32::max);
            let active_frame = frame_peak > 0.002;

            if let Ok(mut g) = self.bands.lock() {
                let elapsed = g.updated_at.elapsed();
                let elapsed_hops = elapsed.as_secs_f32() * self.sample_rate / HOP_SIZE as f32;
                let decay = DECAY_FACTOR.powf(elapsed_hops.max(0.1));
                let peak_decay = DECAY_FACTOR_PEAK.powf(elapsed_hops.max(0.1));
                for (stored, fresh) in g.values.iter_mut().zip(self.new_bands.iter()) {
                    *stored = (*stored * decay).max(*fresh);
                }
                g.peak_envelope = (g.peak_envelope * peak_decay).max(frame_peak);
                g.updated_at = Instant::now();
                if active_frame {
                    g.is_active = true;
                } else if g.values.iter().all(|&v| v < 0.004) {
                    g.is_active = false;
                }
            }

            self.sample_buf.drain(..HOP_SIZE);
        }
    }

    pub fn stop(&mut self) {
        if let Ok(mut g) = self.bands.lock() {
            g.values.fill(0.0);
            g.peak_envelope = 1e-6;
            g.updated_at = Instant::now();
            g.is_active = false;
        }
        self.sample_buf.clear();
    }
}

/// A tee'd sink for `AudioTap`: forwards audio to `inner` and computes FFT bands on the side.
pub struct VisualizationSink {
    inner: Box<dyn Sink>,
    fft: FftProcessor,
}

impl VisualizationSink {
    pub fn new(inner: Box<dyn Sink>, bands: Arc<Mutex<VisBands>>, sample_rate: f32) -> Self {
        Self {
            inner,
            fft: FftProcessor::new(bands, sample_rate, 1.0),
        }
    }
}

impl Sink for VisualizationSink {
    fn start(&mut self) -> SinkResult<()> {
        self.inner.start()
    }

    fn stop(&mut self) -> SinkResult<()> {
        self.fft.stop();
        self.inner.stop()
    }

    fn write(&mut self, packet: AudioPacket) -> SinkResult<()> {
        let AudioPacket::Samples(ref samples) = packet;
        let mono: Vec<f32> = samples
            .chunks(2)
            .map(|c| {
                if c.len() == 2 {
                    f64::midpoint(c[0], c[1]) as f32
                } else {
                    c[0] as f32
                }
            })
            .collect();
        self.fft.process_mono_samples(&mono);
        self.inner.write(packet)
    }
}

/// PCM analyzer that bridges PipeWire audio streams into the FFT analyzer.
pub struct WaveAnalyzer {
    pub rate: u32,
    pub channels: u32,
    pub fft_processor: FftProcessor,
    status_tx: watch::Sender<VisualizerStatus>,
}

impl WaveAnalyzer {
    pub fn new(
        bands: Arc<Mutex<VisBands>>,
        sensitivity: f32,
        status_tx: watch::Sender<VisualizerStatus>,
    ) -> Self {
        Self {
            rate: 48_000,
            channels: 2,
            fft_processor: FftProcessor::new(bands, 48_000.0, sensitivity),
            status_tx,
        }
    }

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

        self.fft_processor.process_mono_samples(&mono);
    }
}

/// Spawn the audio observer on a background thread.
pub fn spawn(
    enabled: bool,
    sensitivity: f32,
    target: String,
    bands: Arc<Mutex<VisBands>>,
) -> VisualizerHandle {
    let (status_tx, status_rx) = watch::channel(VisualizerStatus::Starting);
    let stop = Arc::new(AtomicBool::new(false));

    if !enabled {
        let _ = status_tx.send_replace(VisualizerStatus::Disabled("disabled in config".into()));
        return VisualizerHandle {
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
                run_pipewire_observer(sensitivity, target_clone, bands, status_tx, stop_clone);
            });

        if let Err(e) = thread_res {
            return VisualizerHandle::new_disabled(format!("thread spawn failed: {e}"));
        }
    }

    #[cfg(not(all(target_os = "linux", feature = "streaming")))]
    {
        let _ = target;
        let _ = sensitivity;
        let _ = bands;
        let _ = status_tx.send_replace(VisualizerStatus::Unavailable(
            "pipewire observer requires linux with streaming feature".into(),
        ));
    }

    VisualizerHandle {
        status_rx,
        stop,
    }
}

#[cfg(all(target_os = "linux", feature = "streaming"))]
fn run_pipewire_observer(
    sensitivity: f32,
    target: String,
    bands: Arc<Mutex<VisBands>>,
    status_tx: watch::Sender<VisualizerStatus>,
    stop: Arc<AtomicBool>,
) {
    if let Err(err) = run_pipewire_inner(sensitivity, target, bands, &status_tx, &stop) {
        log::debug!("visualizer pipewire observer: {err}");
        let _ = status_tx.send_replace(VisualizerStatus::Unavailable(err.to_string()));
    }
}

#[cfg(all(target_os = "linux", feature = "streaming"))]
fn run_pipewire_inner(
    sensitivity: f32,
    target: String,
    bands: Arc<Mutex<VisBands>>,
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

    let stream = pw::stream::StreamRc::new(core, "myx-visualizer", props)?;

    let analyzer = WaveAnalyzer::new(bands, sensitivity, status_tx.clone());
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
            user_data.fft_processor.sample_rate = rate as f32;
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

fn precompute_band_ranges(num_bins: usize, num_bands: usize) -> Vec<(usize, usize)> {
    let log_min = 1.0_f64;
    let log_max = num_bins as f64;
    let mut used_up_to: usize = 1;
    let mut ranges = Vec::with_capacity(num_bands);
    for band in 0..num_bands {
        if used_up_to >= num_bins {
            ranges.push((num_bins - 1, num_bins));
            continue;
        }
        let t_start = band as f64 / num_bands as f64;
        let t_end = (band + 1) as f64 / num_bands as f64;
        let natural_start = (log_min * (log_max / log_min).powf(t_start)) as usize;
        let natural_end = (log_min * (log_max / log_min).powf(t_end)) as usize;
        let start = natural_start.max(used_up_to).min(num_bins - 1);
        let end = natural_end.max(start + 1).min(num_bins);
        used_up_to = end;
        ranges.push((start, end));
    }
    ranges
}

fn fill_log_bands(magnitudes: &[f32], band_ranges: &[(usize, usize)], out: &mut [f32]) {
    for (band_val, &(start, end)) in out.iter_mut().zip(band_ranges.iter()) {
        let len = (end - start) as f32;
        let sum_sq: f32 = magnitudes[start..end].iter().map(|&v| v * v).sum();
        *band_val = (sum_sq / len).sqrt();
    }
}

fn smooth_bands(bands: &mut [f32], scratch: &mut [f32]) {
    let n = bands.len();
    if n < 3 {
        return;
    }
    scratch[..n].copy_from_slice(&bands[..n]);
    for i in 0..n {
        let prev = scratch[if i > 0 { i - 1 } else { 0 }];
        let next = scratch[if i + 1 < n { i + 1 } else { n - 1 }];
        bands[i] = prev * 0.25 + scratch[i] * 0.5 + next * 0.25;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precompute_band_ranges_valid() {
        let ranges = precompute_band_ranges(512, NUM_BANDS);
        assert_eq!(ranges.len(), NUM_BANDS);
        for &(start, end) in &ranges {
            assert!(start < end);
            assert!(end <= 512);
        }
    }

    #[test]
    fn fft_processor_processes_samples_and_decays() {
        let bands = VisBands::shared();
        let mut proc = FftProcessor::new(Arc::clone(&bands), 44100.0, 1.0);
        let samples = vec![0.5f32; 2048];
        proc.process_mono_samples(&samples);

        let g = bands.lock().unwrap();
        assert!(g.is_active);
        assert!(g.peak_envelope > 0.0);
    }
}
