//! Soloist process and WebSocket integration.
//!
//! Soloist is the Spotify Connect receiver myx drives:
//! it owns authentication, decoding and playback. Myx
//! starts one when needed, discovers its control
//! WebSocket and translates commands and events.

pub mod client;
pub mod discovery;
pub mod process;
pub mod protocol;
