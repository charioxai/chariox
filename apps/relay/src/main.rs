use std::collections::BTreeMap;

use arroba_relay::{RelayAuthVerifier, RelayConfig, RelayServer};
use serde_json::json;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = RelayConfig::load_from_env()?;
    let scoped_verifier = scoped_verifier_from_env();
    if let Some(message) = open_access_startup_error(
        &config.host,
        config.shared_token.is_some(),
        scoped_verifier.is_some(),
        parse_env_flag(
            std::env::var("ARROBA_RELAY_ALLOW_OPEN_ACCESS")
                .ok()
                .as_deref(),
        ),
    ) {
        eprintln!(
            "{}",
            json!({
                "component": "arroba-relay",
                "level": "error",
                "event": "relay_open_access_refused",
                "fields": { "host": config.host, "message": message },
            })
        );
        return Err(message.into());
    }
    let server = if let Some(verifier) = scoped_verifier {
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
    // Optional: pull account/identity revocations from the hosted control
    // plane into this relay's registry so revoked tokens are rejected.
    let _revocation_shutdown_tx = if let Some((cloud_url, realm)) = revocation_sync_from_env() {
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        eprintln!(
            "{}",
            json!({
                "component": "arroba-relay",
                "level": "info",
                "event": "revocation_sync_enabled",
                "fields": { "realm_id": realm },
            })
        );
        tokio::spawn(arroba_relay::revocation_sync::run_revocation_sync(
            cloud_url,
            realm,
            server.revocations(),
            revocation_sync_interval_from_env(),
            shutdown_rx,
        ));
        Some(shutdown_tx)
    } else {
        None
    };
    server.run().await?;
    Ok(())
}

fn revocation_sync_from_env() -> Option<(String, String)> {
    let cloud_url = std::env::var("ARROBA_RELAY_REVOCATION_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    let realm = std::env::var("ARROBA_RELAY_REVOCATION_REALM")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    Some((cloud_url, realm))
}

fn revocation_sync_interval_from_env() -> std::time::Duration {
    std::env::var("ARROBA_RELAY_REVOCATION_INTERVAL_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map(std::time::Duration::from_secs)
        .unwrap_or(arroba_relay::revocation_sync::DEFAULT_REVOCATION_SYNC_INTERVAL)
}

fn parse_relay_draining(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("1" | "true" | "yes" | "on" | "draining")
    )
}

fn parse_env_flag(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

// A relay bound beyond loopback with no verifier at all would accept every
// request; that must be an explicit operator decision, never a silent default.
fn open_access_startup_error(
    host: &str,
    has_shared_token: bool,
    has_scoped_verifier: bool,
    allow_open_access: bool,
) -> Option<String> {
    if has_shared_token || has_scoped_verifier || allow_open_access {
        return None;
    }
    if host_is_loopback(host) {
        return None;
    }
    Some(format!(
        "refusing to start an unauthenticated relay on non-loopback host `{host}`; \
         set ARROBA_RELAY_TOKEN, configure a scoped issuer, or explicitly opt in \
         with ARROBA_RELAY_ALLOW_OPEN_ACCESS=1"
    ))
}

fn host_is_loopback(host: &str) -> bool {
    let host = host.trim();
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|address| address.is_loopback())
        .unwrap_or(false)
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

    #[test]
    fn open_access_requires_explicit_opt_in_on_non_loopback_hosts() {
        assert!(open_access_startup_error("0.0.0.0", false, false, false).is_some());
        assert!(open_access_startup_error("192.168.1.10", false, false, false).is_some());
        assert!(open_access_startup_error("relay.example.com", false, false, false).is_some());
        assert!(open_access_startup_error("0.0.0.0", false, false, true).is_none());
    }

    #[test]
    fn configured_auth_or_loopback_hosts_start_without_opt_in() {
        assert!(open_access_startup_error("0.0.0.0", true, false, false).is_none());
        assert!(open_access_startup_error("0.0.0.0", false, true, false).is_none());
        assert!(open_access_startup_error("127.0.0.1", false, false, false).is_none());
        assert!(open_access_startup_error("::1", false, false, false).is_none());
        assert!(open_access_startup_error("localhost", false, false, false).is_none());
    }
}
