# soloist-tui

A lean, beautiful terminal Spotify player in Rust, powered by **Soloist**. Streams natively as a Spotify Connect device with album-art-reactive dynamic theming, a liquid waveform ribbon visualizer, a ten-band parametric equalizer, synced lyrics, full queue management, and like track indication.

<p align="center">
  <video src="https://github.com/user-attachments/assets/90706ba9-4c48-43d0-ad16-17b95c95dc94" alt="soloist-tui recolors the whole interface to the album art" width="100%"></video>
</p>

<p align="center">
  <img src="https://github.com/HaseebKhalid1507/Myx/releases/download/readme-assets/myx.png" width="49%" alt="soloist-tui playback view">
  <img src="https://github.com/user-attachments/assets/48dfec67-edd1-4903-a18d-8ed6d06ee5f9" width="49%" alt="soloist-tui library and search">
  <img src="https://github.com/user-attachments/assets/dd3844ad-f0c7-41ec-a934-86bb0e3ef75b" width="49%" alt="soloist-tui equalizer overlay">
  <img src="https://github.com/user-attachments/assets/08b3f505-5e48-4cd8-9b8d-0788d37f30c2" width="49%" alt="soloist-tui synced lyrics">
</p>

> **Note:** Requires **Spotify Premium**. Runs on Linux, macOS, and Windows. Album art renders crispest on modern terminal emulators with graphics protocol support (Kitty, WezTerm, Ghostty, or Foot).

---

## ✨ Features

- ⚡ **Soloist Backend Integration**: High-performance, low-latency Spotify Connect engine driven over local WebSocket with real-time state tracking and automatic session reconnection.
- 🎨 **Album-Art-Reactive Theming**: Real-time extraction of a 16-token semantic color palette from album covers with smooth interpolated cross-fades between tracks.
- 🌊 **Liquid Waveform Ribbon Visualizer**: High-resolution center-axis fluid audio ribbon visualizer responding dynamically to active sound playback.
- 🎚️ **10-Band Live Parametric Equalizer**: 10 frequency bands (31Hz – 16kHz) with presets (*Bass Boost*, *Rock*, *Jazz*, *Vocal*, *Electronic*, *Treble Boost*, *Flat*) and custom curves with digital headroom management.
- 📜 **Synced & Plain Lyrics**: Automatically synchronizes and auto-scrolls lyrics via LRCLib, keeping pace with current playback position.
- ❤️ **Full UI Liked Track Indicators**: Instant `♥` badges across the library, drill-in views, search results, queue, persistent now-playing bar, and Now Playing screen; one-key toggle (`l`) with immediate optimistic feedback and Spotify Web API synchronization.
- 📑 **Comprehensive Library & Queue**: Multi-section browser (Playlists, Liked Songs, Albums, Artists, Recently Played, and Home feed), interactive search, and live queue inspection.
- 🪟 **Transparent Terminal Background**: Press `T` anytime to toggle transparent background mode so your terminal's blur, opacity, or wallpaper shines through.
- 🎧 **OS Media Controls & MPRIS**: Native integration with system media keys, lock screen widgets, and headphone controls on macOS and Linux.
- 🔌 **MXC (Color Protocol)**: Exposes a Unix domain socket (`$XDG_RUNTIME_DIR/soloist-tui/theme.sock`) streaming album-derived palette tokens for external scripting, window manager borders, Polybar/Waybar integration, or desktop theming.

---

## 📦 Requirements & System Dependencies

### 1. Spotify Requirements
- A **Spotify Premium** account (required by Spotify for streaming via Connect).
- A free **Spotify Developer Client ID** (see [Setup](#-getting-started)).

### 2. Audio & Build Dependencies

#### **Linux (Debian / Ubuntu / Pop!_OS / Linux Mint)**
```bash
sudo apt update
sudo apt install -y build-essential pkg-config libasound2-dev libssl-dev
```

#### **Linux (Arch Linux / Manjaro)**
```bash
sudo pacman -S --needed base-devel alsa-lib openssl
```

#### **Linux (Fedora / RHEL)**
```bash
sudo dnf install -y gcc pkg-config alsa-lib-devel openssl-devel
```

#### **macOS**
- Xcode Command Line Tools: `xcode-select --install`
- Audio works out of the box via macOS CoreAudio.

#### **Windows**
- Install [Rust via rustup.rs](https://rustup.rs).
- Native Windows audio and terminal console support are included.

### 3. Spotify Soloist Binary
`soloist-tui` drives **Spotify Soloist** as its playback engine.
Ensure `soloist` is installed in your `$PATH` (or specify its location in `~/.config/soloist-tui/config.toml` under `[soloist] executable`).

---

## 🚀 Installation

### From Source (Cargo)
Ensure you have the Rust toolchain installed:

```bash
git clone https://github.com/HaseebKhalid1507/soloist-tui.git
cd soloist-tui
cargo install --path .
```

The compiled binary will be placed at `~/.cargo/bin/soloist-tui`.

### Build Release Binary
To build an optimized release binary locally:

```bash
cargo build --release
# Executable is located at target/release/soloist-tui
```

---

## 🔑 Getting Started

`soloist-tui` uses Spotify's modern OAuth 2.0 PKCE authentication flow. You only need a free Spotify Client ID (takes about 1 minute):

1. Go to the [Spotify Developer Dashboard](https://developer.spotify.com/dashboard) and log in.
2. Click **Create App**:
   - **App Name**: `soloist-tui` (or any name you like)
   - **App Description**: `Terminal Spotify player`
   - **Redirect URI**: `http://127.0.0.1:8989/login` *(Must match exactly)*
   - **APIs used**: Select **Web API**
3. Save the app, open **Settings**, and copy your **Client ID**.
4. Set your Client ID either:
   - Via environment variable:
     ```bash
     export SOLOIST_CLIENT_ID="<your-32-digit-client-id>"
     ```
   - Or in `~/.config/soloist-tui/config.toml`:
     ```toml
     client_id = "<your-32-digit-client-id>"
     ```
   - Or simply launch `soloist-tui` and paste it into the interactive prompt on first run!

5. Run `soloist-tui`:
   ```bash
   soloist-tui
   ```
   On first launch, your browser will open to authorize your client. Once authorized, you can close the browser tab. All subsequent launches will authenticate automatically.

---

## ⌨️ Keybindings

| Key | Action |
| :--- | :--- |
| `⇥` / `[` `]` | Cycle library sections (Home, Recent, Playlists, Liked, Albums, Artists) |
| `←` `→` | Switch active right-hand view (*Now Playing*, *Lyrics*, *Queue*) |
| `↑` `↓` / `j` `k` | Move cursor in the active list |
| `⏎` (Enter) | Open selected context / play selected track |
| `⇧ ⏎` / `P` | Play context outright (plays selected playlist/album/artist) |
| `S` | Play selected context with shuffle enabled |
| `l` | **Toggle Liked status** for selected track (or currently playing track) |
| `/` | Open search input (press `⏎` to search, `Esc` to cancel) |
| `Space` / `p` | Play / Pause toggle |
| `n` / `b` | Skip to next / previous track |
| `⇧ ←` / `⇧ →` | Seek backward / forward 5 seconds |
| `+` / `-` | Volume up / down |
| `s` | Toggle shuffle mode |
| `R` | Cycle repeat mode (Off / Context / Track) |
| `o` | Cycle sort order (Added, Title, Artist) |
| `a` | Context actions menu (Add to queue, playlist, copy link, go to artist/album) |
| `e` | Open 10-band Equalizer overlay |
| `z` | Toggle Zen mode (collapses library sidebar) |
| `T` | Toggle transparent background |
| `r` | Refresh library sections |
| `q` / `Ctrl-C` | Quit |

---

## ⚙️ Configuration

`soloist-tui` automatically generates a configuration template on its first run at `~/.config/soloist-tui/config.toml` (with full backwards compatibility for `~/.config/myx/config.toml` if upgrading).

```toml
# soloist-tui configuration

# Number of rows kept visible above and below the cursor when scrolling
scrolloff = 3

# Restore the last played track, context, queue, and position on startup (paused)
restore_on_startup = true

# Your Spotify Developer App Client ID
# (Overrides can also be supplied via the SOLOIST_CLIENT_ID environment variable)
client_id = "your-client-id-here"

# Terminal graphics protocol for album art: "kitty", "iterm2", "sixel", or "halfblocks"
# Omit or leave commented to auto-detect your terminal capabilities
protocol = "kitty"

# Streaming bitrate: 96, 160, or 320 kbps (defaults to 160)
bitrate = 160

# Normalizes volume across tracks (similar to official Spotify client)
normalize_volume = false

# Enable transparent background (inherits terminal background color and blur)
transparent = true

# Liquid waveform ribbon visualizer
visualizer_enabled = true
visualizer_sensitivity = 1.0

# Soloist playback engine configuration
[soloist]
device_name = "soloist-tui"
executable = "soloist"
route_audio = false
# ws = "ws://127.0.0.1:24879" # Optional: specify explicit running Soloist WebSocket endpoint
```

### Environment Variables
- `SOLOIST_CLIENT_ID` (or `MYX_CLIENT_ID`): Overrides `client_id` in config.toml.
- `SOLOIST_CONFIG` (or `MYX_CONFIG`): Specify custom path to `config.toml`.
- `SOLOIST_DEVICE_NAME`: Overrides Spotify Connect device name.
- `SOLOIST_NO_COLOR_SOCKET`: Set to `1` to disable the MXC color publishing socket.
- `SOLOIST_PROTOCOL`: Force terminal image protocol (`kitty`, `iterm2`, `sixel`, `halfblocks`).

---

## 🎨 MXC (Color Protocol) Integration

`soloist-tui` publishes its real-time album art palette over a local Unix domain socket at `$XDG_RUNTIME_DIR/soloist-tui/theme.sock`. External scripts or window managers can subscribe to these colors live:

```bash
# Print current theme palette variables and exit
soloist-tui theme get

# Export directly into current shell
eval "$(soloist-tui theme get)"
echo $MYX_PRIMARY $MYX_ACCENT

# Stream CSS custom properties on track changes
soloist-tui theme watch --format css

# Trigger an external reload script on each track change
soloist-tui theme watch --exec "polybar-msg hook theme-reload 1"
```

---

## 📜 Credits & License

- Built on the awesome [ratatui](https://ratatui.rs) ecosystem.
- Connected via [Spotify Soloist](https://github.com/mopidy/soloist).
- Visual design language and design-token aesthetics inspired by [noodle](https://github.com/wilfredinni/noodle).
- Lyrics powered by [LRCLib](https://lrclib.net).
- Audio DSP and equalizer powered by [rodio](https://github.com/RustAudio/rodio) and [rustfft](https://github.com/ejmahler/RustFFT).

Licensed under the [MIT License](LICENSE).
