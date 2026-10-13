//! Endpoint discovery, exactly as Soloist documents it: read `ws.addr`
//! and `ws.port` from the Soloist data directory.
//!
//! Data directory resolution matches Soloist's own order:
//! `$STATE_DIRECTORY` (first entry, systemd) → `$XDG_DATA_HOME/soloist`
//! → `~/.local/share/soloist`.

use std::path::{Path, PathBuf};

use crate::home_dir;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

impl Endpoint {
    pub fn ws_url(&self) -> String {
        // Hosts that are not already bracketed need brackets in a URL.
        let host = if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        format!("ws://{host}:{}", self.port)
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        let (host, port) = s
            .rsplit_once(':')
            .ok_or_else(|| format!("expected ADDR:PORT, got {s:?}"))?;
        let port: u16 = port.parse().map_err(|_| format!("invalid port in {s:?}"))?;
        if host.is_empty() {
            return Err(format!("missing host in {s:?}"));
        }
        Ok(Self {
            host: host.trim_matches(['[', ']']).to_string(),
            port,
        })
    }
}

/// Resolve the Soloist data directory using Soloist's documented
/// precedence.
pub fn default_data_dir() -> PathBuf {
    if let Ok(state) = std::env::var("STATE_DIRECTORY") {
        // systemd sets a colon-separated list; Soloist's state dir is
        // first.
        if let Some(first) = state.split(':').next() {
            if !first.is_empty() {
                return PathBuf::from(first);
            }
        }
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("soloist");
        }
    }
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local/share/soloist")
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Discover the running Soloist WebSocket endpoint from runtime files.
pub fn discover(data_dir: &Path) -> Result<Endpoint, String> {
    let addr = read_trimmed(&data_dir.join("ws.addr"))
        .ok_or("no ws.addr in the Soloist data directory")?;
    let port_raw = read_trimmed(&data_dir.join("ws.port"))
        .ok_or("no ws.port in the Soloist data directory")?;
    let port: u16 = port_raw
        .parse()
        .map_err(|_| format!("invalid ws.port contents: {port_raw:?}"))?;

    // ws.addr may be a bare address or already host:port tolerant.
    let host = addr
        .rsplit_once(':')
        .map(|(h, _)| h.trim_matches(['[', ']']).to_string())
        .unwrap_or_else(|| addr.trim_matches(['[', ']']).to_string());

    Ok(Endpoint { host, port })
}

/// True when Soloist's pid file exists (used for clearer error
/// messages).
pub fn daemon_running(data_dir: &Path) -> bool {
    data_dir.join("soloist.pid").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_roundtrip() {
        let ep = Endpoint::parse("127.0.0.1:9090").unwrap();
        assert_eq!(ep.host, "127.0.0.1");
        assert_eq!(ep.port, 9090);
        assert_eq!(ep.ws_url(), "ws://127.0.0.1:9090");
        assert_eq!(ep, Endpoint::parse(&ep.to_string()).unwrap());
    }

    #[test]
    fn ipv6_url_is_bracketed() {
        let ep = Endpoint::parse("::1:9090").unwrap();
        assert_eq!(ep.host, "::1");
        assert_eq!(ep.ws_url(), "ws://[::1]:9090");
    }

    #[test]
    fn bad_endpoint_is_rejected() {
        assert!(Endpoint::parse("no-port").is_err());
        assert!(Endpoint::parse("host:notaport").is_err());
        assert!(Endpoint::parse(":9090").is_err());
    }

    #[test]
    fn discovers_from_runtime_files() {
        let dir = std::env::temp_dir().join(format!("myx-soloist-disc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ws.addr"), "127.0.0.1").unwrap();
        std::fs::write(dir.join("ws.port"), "41277\n").unwrap();
        let ep = discover(&dir).unwrap();
        assert_eq!(ep.port, 41277);
        assert!(!daemon_running(&dir));

        std::fs::remove_file(dir.join("ws.port")).unwrap();
        assert!(discover(&dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
