//! Audio engine internals (streaming feature).

pub mod backend;
pub mod equalizer;
pub mod tap;

pub use backend::{AudioPacket, RodioSink, Sink, SinkResult};
pub(crate) use equalizer::{shared_equalizer, EqualizerControl};
pub use equalizer::{
    EqualizerPreset, EqualizerSettings, EQ_FREQUENCIES_HZ, MAX_EQ_GAIN_DB, MIN_EQ_GAIN_DB,
    NUM_EQ_BANDS,
};
pub use tap::{AudioTap, SinkChain};

#[cfg(all(target_os = "linux", feature = "streaming"))]
pub use backend::suppress_alsa_errors;
