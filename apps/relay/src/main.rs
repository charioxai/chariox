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
    let draining = parse_relay_draining(std::env::var("ARROBA_RELAY_DRAINING").ok().as_deref());
    server.set_draining(draining);
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
                "draining": draining,
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

fn parse_relay_draining(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("1" | "true" | "yes" | "on" | "draining")
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_relay_draining_accepts_only_explicit_truthy_values() {
        for value in ["1", "true", "TRUE", " yes ", "on", "draining"] {
            assert!(
                parse_relay_draining(Some(value)),
                "{value} should enable draining"
            );
        }
        for value in [None, Some(""), Some("0"), Some("false"), Some("healthy")] {
            assert!(!parse_relay_draining(value));
        }
    }
}
