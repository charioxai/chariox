use std::collections::BTreeMap;

use arroba_relay::{RelayAuthVerifier, RelayConfig, RelayServer};
use serde_json::json;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = RelayConfig::load_from_env()?;
    let server = if let Some(verifier) = scoped_verifier_from_env() {
        RelayServer::with_auth_verifier(config.clone(), verifier)
    } else {
        RelayServer::new(config.clone())
    };
    eprintln!(
        "{}",
        json!({
            "component": "arroba-relay",
            "level": "info",
            "event": "relay_process_starting",
            "fields": {
                "host": config.host,
                "port": config.port,
                "package_version": env!("CARGO_PKG_VERSION"),
                "build_commit": std::env::var("ARROBA_BUILD_COMMIT").ok(),
                "scoped_verifier": std::env::var("ARROBA_RELAY_SCOPED_ISSUER")
                    .ok()
                    .map(|value| !value.trim().is_empty())
                    .unwrap_or(false),
            },
        })
    );
    server.run().await?;
    Ok(())
}

fn scoped_verifier_from_env() -> Option<RelayAuthVerifier> {
    let issuer = std::env::var("ARROBA_RELAY_SCOPED_ISSUER")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    let secret = std::env::var("ARROBA_RELAY_SCOPED_HMAC_SECRET")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    Some(RelayAuthVerifier::scoped_hmac(
        BTreeMap::from([(issuer, secret)]),
        None,
    ))
}
