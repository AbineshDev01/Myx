//! Wire types for the Spotify Soloist WebSocket API.
//!
//! Every type mirrors the official reference:
//! <https://developer.spotify.com/documentation/soloist/reference/websocket-api>
//! Nothing is invented; unknown fields are ignored so new Soloist
//! releases do not break the client.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Entities
// ---------------------------------------------------------------------------

/// Common entity envelope used by playback items, contexts and queue
/// entries.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct Entity {
    #[serde(default)]
    pub uri: String,
    #[serde(default, rename = "entity_type")]
    pub entity_type: String,
    #[serde(default)]
    pub decorations: Decorations,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct Decorations {
    #[serde(default)]
    pub identity: Option<Identity>,
    #[serde(default)]
    pub visual_identity: Option<VisualIdentity>,
    #[serde(default)]
    pub parent: Option<Parent>,
    #[serde(default)]
    pub creators: Vec<Creator>,
    #[serde(default)]
    pub playback: Option<PlaybackDecorations>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct Identity {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct VisualIdentity {
    #[serde(default)]
    pub cover: Vec<CoverImage>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct CoverImage {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub size: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct Parent {
    #[serde(default)]
    pub entity: Box<Entity>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct Creator {
    #[serde(default)]
    pub entity: Box<Entity>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct PlaybackDecorations {
    #[serde(default, rename = "duration_ms")]
    pub duration_ms: Option<u64>,
    #[serde(default, rename = "content_ratings")]
    pub content_ratings: Vec<String>,
}

impl Entity {
    pub fn name(&self) -> Option<&str> {
        self.decorations
            .identity
            .as_ref()
            .and_then(|i| i.name.as_deref())
            .filter(|s| !s.is_empty())
    }

    pub fn album_name(&self) -> Option<&str> {
        self.decorations
            .parent
            .as_ref()
            .and_then(|p| p.entity.name())
    }

    pub fn artist_names(&self) -> Vec<String> {
        self.decorations
            .creators
            .iter()
            .filter_map(|c| c.entity.name().map(str::to_owned))
            .collect()
    }

    pub fn duration_ms(&self) -> Option<u64> {
        self.decorations
            .playback
            .as_ref()
            .and_then(|p| p.duration_ms)
    }

    pub fn is_explicit(&self) -> bool {
        self.decorations
            .playback
            .as_ref()
            .map(|p| p.content_ratings.iter().any(|r| r == "explicit"))
            .unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// Playback primitives
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackStatus {
    Idle,
    Playing,
    Paused,
    Buffering,
    #[default]
    #[serde(other)]
    Unknown,
}

impl PlaybackStatus {
    pub fn is_playing(self) -> bool {
        matches!(self, Self::Playing | Self::Buffering)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RepeatMode {
    #[default]
    Off,
    Context,
    Track,
    #[serde(other)]
    Unknown,
}

impl RepeatMode {
    /// Display label used by the UI (`off` / `all` / `one`).
    pub fn label(self) -> &'static str {
        match self {
            Self::Off | Self::Unknown => "OFF",
            Self::Context => "ALL",
            Self::Track => "ONE",
        }
    }

    /// Next mode in the cycle used by the `r` key.
    pub fn next(self) -> Self {
        match self {
            Self::Off | Self::Unknown => Self::Context,
            Self::Context => Self::Track,
            Self::Track => Self::Off,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct Position {
    #[serde(default, rename = "position_ms")]
    pub position_ms: u64,
    #[serde(default, rename = "timestamp_ms")]
    pub timestamp_ms: i64,
    #[serde(default)]
    pub speed: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct PlaybackOptions {
    #[serde(default)]
    pub shuffle: bool,
    #[serde(default)]
    pub repeat: RepeatMode,
    #[serde(default, rename = "playback_speed")]
    pub playback_speed: Option<f64>,
    #[serde(default)]
    pub modes: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Server events
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AuthState {
    #[serde(default, rename = "logged_in")]
    pub logged_in: bool,
    #[serde(default, rename = "is_active")]
    pub is_active: bool,
    #[serde(default, rename = "device_name")]
    pub device_name: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct PlaybackState {
    #[serde(default)]
    pub status: PlaybackStatus,
    #[serde(default)]
    pub item: Option<Entity>,
    #[serde(default)]
    pub context: Option<Entity>,
    #[serde(default)]
    pub position: Option<Position>,
    #[serde(default)]
    pub volume: Option<u8>,
    #[serde(default, rename = "is_active")]
    pub is_active: Option<bool>,
    #[serde(default)]
    pub options: Option<PlaybackOptions>,
    #[serde(default, rename = "available_actions")]
    pub available_actions: Option<serde_json::Value>,
}

impl PlaybackState {
    pub fn can_seek(&self) -> bool {
        self.available_actions
            .as_ref()
            .and_then(|a| a.get("seek"))
            .is_some()
    }
}


#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct QueueEntry {
    #[serde(default)]
    pub uid: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub item: Entity,
}

/// Every server → client message. Tagged by the `type` field.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
pub enum ServerEvent {
    #[serde(rename = "auth_state")]
    AuthState(AuthState),

    #[serde(rename = "playback_state")]
    PlaybackState(PlaybackState),

    #[serde(rename = "track_changed")]
    TrackChanged { item: Entity },

    #[serde(rename = "playback_changed")]
    PlaybackChanged { status: PlaybackStatus },

    #[serde(rename = "volume_changed")]
    VolumeChanged { volume: u8 },

    #[serde(rename = "device_changed")]
    DeviceChanged {
        #[serde(default, rename = "is_active")]
        is_active: bool,
        #[serde(default, rename = "device_name")]
        device_name: String,
    },

    #[serde(rename = "context_changed")]
    ContextChanged { context: Entity },

    #[serde(rename = "options_changed")]
    OptionsChanged { options: PlaybackOptions },

    #[serde(rename = "position_sync")]
    PositionSync { position: Position },

    #[serde(rename = "queue_changed")]
    QueueChanged {
        #[serde(default)]
        previous: Vec<QueueEntry>,
        #[serde(default)]
        upcoming: Vec<QueueEntry>,
    },

    #[serde(rename = "command_result")]
    CommandResult { command: String },

    #[serde(rename = "error")]
    Error { message: String },
}

impl ServerEvent {
    /// Parse a raw text frame. Returns `Ok(None)` for unknown event
    /// types so future Soloist additions are tolerated instead of
    /// being fatal.
    pub fn parse(frame: &str) -> Result<Option<Self>, serde_json::Error> {
        let value: serde_json::Value = serde_json::from_str(frame)?;
        let Some(kind) = value.get("type").and_then(|t| t.as_str()) else {
            return Ok(None);
        };
        // Only messages that follow the documented shape are attempted.
        if !KNOWN_EVENT_TYPES.contains(&kind) {
            log::debug!(target: "myx::soloist", "ignoring unknown Soloist event: {kind}");
            return Ok(None);
        }
        serde_json::from_value(value).map(Some)
    }
}

const KNOWN_EVENT_TYPES: &[&str] = &[
    "auth_state",
    "playback_state",
    "track_changed",
    "playback_changed",
    "volume_changed",
    "device_changed",
    "context_changed",
    "options_changed",
    "position_sync",
    "queue_changed",
    "command_result",
    "error",
];

// ---------------------------------------------------------------------------
// Client commands
// ---------------------------------------------------------------------------

/// Client → server message. Always `{"type":"command", "command": ..., ...}`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Command {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub command: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

impl Command {
    fn new(command: &'static str) -> Self {
        Self {
            kind: "command",
            command,
            uri: None,
            volume: None,
            enabled: None,
            position_ms: None,
            limit: None,
        }
    }

    pub fn get_auth_state() -> Self {
        Self::new("get_auth_state")
    }
    pub fn get_state() -> Self {
        Self::new("get_state")
    }
    pub fn get_queue(limit: Option<u32>) -> Self {
        let mut c = Self::new("get_queue");
        c.limit = limit;
        c
    }
    pub fn play() -> Self {
        Self::new("play")
    }
    pub fn play_uri(uri: impl Into<String>) -> Self {
        let mut c = Self::new("play");
        c.uri = Some(uri.into());
        c
    }
    pub fn pause() -> Self {
        Self::new("pause")
    }
    pub fn skip_next() -> Self {
        Self::new("skip_next")
    }
    pub fn skip_prev() -> Self {
        Self::new("skip_prev")
    }
    pub fn seek(position_ms: u64) -> Self {
        let mut c = Self::new("seek");
        c.position_ms = Some(position_ms);
        c
    }
    pub fn set_volume(volume: u8) -> Self {
        let mut c = Self::new("set_volume");
        c.volume = Some(volume.clamp(0, 100));
        c
    }
    pub fn set_shuffle(enabled: bool) -> Self {
        let mut c = Self::new("set_shuffle");
        c.enabled = Some(enabled);
        c
    }
    pub fn set_repeat_context(enabled: bool) -> Self {
        let mut c = Self::new("set_repeat_context");
        c.enabled = Some(enabled);
        c
    }
    pub fn set_repeat_track(enabled: bool) -> Self {
        let mut c = Self::new("set_repeat_track");
        c.enabled = Some(enabled);
        c
    }
    pub fn add_to_queue(uri: impl Into<String>) -> Self {
        let mut c = Self::new("add_to_queue");
        c.uri = Some(uri.into());
        c
    }
    pub fn activate() -> Self {
        Self::new("activate")
    }
    pub fn deactivate() -> Self {
        Self::new("deactivate")
    }

    /// Repeat mode is expressed as two booleans. The official reference
    /// documents this exact ordering.
    pub fn repeat_mode(mode: RepeatMode) -> Vec<Self> {
        match mode {
            RepeatMode::Off | RepeatMode::Unknown => vec![
                Self::set_repeat_track(false),
                Self::set_repeat_context(false),
            ],
            RepeatMode::Context => vec![
                Self::set_repeat_track(false),
                Self::set_repeat_context(true),
            ],
            RepeatMode::Track => vec![
                Self::set_repeat_context(false),
                Self::set_repeat_track(true),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYBACK_STATE_JSON: &str = r#"{
        "type": "playback_state",
        "status": "playing",
        "item": {
            "uri": "spotify:track:2JRo0gjbX4GrCqBYdRohoo",
            "entity_type": "track",
            "decorations": {
                "identity": { "name": "My Song" },
                "visual_identity": { "cover": [ { "url": "https://i.scdn.co/image/x", "size": "large" } ] },
                "parent": { "entity": { "uri": "spotify:album:4", "entity_type": "album",
                    "decorations": { "identity": { "name": "Album Name" } } } },
                "creators": [ { "entity": { "uri": "spotify:artist:1", "entity_type": "artist",
                    "decorations": { "identity": { "name": "Artist Name" } } } } ],
                "playback": { "duration_ms": 210000, "content_ratings": [] }
            }
        },
        "context": { "uri": "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M", "entity_type": "playlist",
            "decorations": { "identity": { "name": "Today's Top Hits" } } },
        "position": { "position_ms": 45000, "timestamp_ms": 1747654321000, "speed": 1.0 },
        "volume": 65,
        "is_active": true,
        "options": { "shuffle": false, "repeat": "off", "playback_speed": 1.0, "modes": {} },
        "available_actions": { "pause": {}, "seek": {}, "seek_forward": { "step_ms": 15000 } }
    }"#;

    #[test]
    fn parses_documented_playback_state() {
        let ev = ServerEvent::parse(PLAYBACK_STATE_JSON).unwrap().unwrap();
        let ServerEvent::PlaybackState(state) = ev else {
            panic!("expected playback_state");
        };
        assert_eq!(state.status, PlaybackStatus::Playing);
        let item = state.item.unwrap();
        assert_eq!(item.name(), Some("My Song"));
        assert_eq!(item.album_name(), Some("Album Name"));
        assert_eq!(item.artist_names(), vec!["Artist Name".to_string()]);
        assert_eq!(item.duration_ms(), Some(210000));
        assert_eq!(state.volume, Some(65));
        let pos = state.position.unwrap();
        assert_eq!(pos.position_ms, 45000);
        assert_eq!(pos.speed, 1.0);
        let opts = state.options.unwrap();
        assert!(!opts.shuffle);
        assert_eq!(opts.repeat, RepeatMode::Off);
    }

    #[test]
    fn parses_auth_state() {
        let ev = ServerEvent::parse(
            r#"{ "type": "auth_state", "logged_in": true, "is_active": true, "device_name": "Kitchen speaker" }"#,
        )
        .unwrap()
        .unwrap();
        let ServerEvent::AuthState(a) = ev else {
            panic!("expected auth_state");
        };
        assert!(a.logged_in);
        assert_eq!(a.device_name, "Kitchen speaker");
    }

    #[test]
    fn parses_all_documented_granular_events() {
        let frames = [
            r#"{ "type": "track_changed", "item": { "uri": "spotify:track:1", "entity_type": "track", "decorations": {} } }"#,
            r#"{ "type": "playback_changed", "status": "paused" }"#,
            r#"{ "type": "volume_changed", "volume": 42 }"#,
            r#"{ "type": "device_changed", "is_active": true, "device_name": "Kitchen speaker" }"#,
            r#"{ "type": "context_changed", "context": { "uri": "", "entity_type": "", "decorations": {} } }"#,
            r#"{ "type": "options_changed", "options": { "shuffle": true, "repeat": "context" } }"#,
            r#"{ "type": "position_sync", "position": { "position_ms": 45000, "timestamp_ms": 1747654321000, "speed": 1.0 } }"#,
            r#"{ "type": "queue_changed", "previous": [], "upcoming": [ { "uid": "u", "source": "queue", "item": { "uri": "spotify:track:2", "entity_type": "track", "decorations": { "identity": { "name": "Next Song" } } } } ] }"#,
            r#"{ "type": "command_result", "command": "pause" }"#,
            r#"{ "type": "error", "message": "command requires authentication" }"#,
        ];
        for frame in frames {
            assert!(
                ServerEvent::parse(frame).unwrap().is_some(),
                "failed to parse {frame}"
            );
        }
    }

    #[test]
    fn unknown_event_types_are_tolerated() {
        assert!(
            ServerEvent::parse(r#"{ "type": "some_future_event", "x": 1 }"#)
                .unwrap()
                .is_none()
        );
        assert!(ServerEvent::parse("not json at all").is_err());
    }

    #[test]
    fn serializes_commands_exactly_as_documented() {
        assert_eq!(
            serde_json::to_string(&Command::pause()).unwrap(),
            r#"{"type":"command","command":"pause"}"#
        );
        assert_eq!(
            serde_json::to_string(&Command::set_volume(50)).unwrap(),
            r#"{"type":"command","command":"set_volume","volume":50}"#
        );
        assert_eq!(
            serde_json::to_string(&Command::set_shuffle(true)).unwrap(),
            r#"{"type":"command","command":"set_shuffle","enabled":true}"#
        );
        assert_eq!(
            serde_json::to_string(&Command::seek(30000)).unwrap(),
            r#"{"type":"command","command":"seek","position_ms":30000}"#
        );
        assert_eq!(
            serde_json::to_string(&Command::get_queue(Some(10))).unwrap(),
            r#"{"type":"command","command":"get_queue","limit":10}"#
        );
        assert_eq!(
            serde_json::to_string(&Command::play_uri("spotify:playlist:1")).unwrap(),
            r#"{"type":"command","command":"play","uri":"spotify:playlist:1"}"#
        );
    }

    #[test]
    fn repeat_mode_command_ordering_matches_docs() {
        assert_eq!(
            Command::repeat_mode(RepeatMode::Off),
            vec![
                Command::set_repeat_track(false),
                Command::set_repeat_context(false)
            ]
        );
        assert_eq!(
            Command::repeat_mode(RepeatMode::Context),
            vec![
                Command::set_repeat_track(false),
                Command::set_repeat_context(true)
            ]
        );
        assert_eq!(
            Command::repeat_mode(RepeatMode::Track),
            vec![
                Command::set_repeat_context(false),
                Command::set_repeat_track(true)
            ]
        );
    }

    #[test]
    fn repeat_cycles_off_all_one() {
        let mut mode = RepeatMode::Off;
        mode = mode.next();
        assert_eq!(mode, RepeatMode::Context);
        mode = mode.next();
        assert_eq!(mode, RepeatMode::Track);
        mode = mode.next();
        assert_eq!(mode, RepeatMode::Off);
    }

    #[test]
    fn status_unknown_is_not_playing() {
        let ev = ServerEvent::parse(r#"{ "type": "playback_changed", "status": "weird" }"#)
            .unwrap()
            .unwrap();
        let ServerEvent::PlaybackChanged { status } = ev else {
            panic!();
        };
        assert_eq!(status, PlaybackStatus::Unknown);
        assert!(!status.is_playing());
    }
}
