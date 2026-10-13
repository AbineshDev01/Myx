//! Verify the radio path returns similar tracks.
use myx::engine;
use myx::webapi::WebApi;
use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let webapi = Arc::new(Mutex::new(WebApi::init()?));
    let seed = "spotify:track:0VjIjW4GlUZAMYd2vXMi3b".to_string();
    match engine::radio_tracks(webapi, &seed).await {
        Ok(uris) => {
            println!("radio OK: {} tracks", uris.len());
            for u in uris.iter().take(6) {
                println!("  {u}");
            }
        }
        Err(e) => println!("radio ERROR: {e:#}"),
    }
    Ok(())
}
