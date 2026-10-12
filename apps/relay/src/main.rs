use std::collections::BTreeMap;

use chariox_relay::{RelayAuthVerifier, RelayConfig, RelayServer};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match (args.first().and_then(|arg| arg.to_str()), args.len()) {
        (None, 0) => {}
        (Some("--help" | "-h"), 1) => {
            println!("usage: chariox-relay [--help | --version]\n\nWith no arguments, run the relay. Configure it through CHARIOX_RELAY_HOST,\nCHARIOX_RELAY_PORT and the relay authentication environment settings.\n  -h, --help  Print usage and exit\n  --version   Print the package version and exit");
            return Ok(());
        }
        (Some("--version"), 1) => {
            println!("chariox-relay {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        _ => {
            eprintln!(
                "error: unknown or additional relay argument\nRun chariox-relay --help for usage."
            );
            std::process::exit(2);
        }
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main())
}

async fn async_main() -> Result<(), Box<dyn std::error::Error>> {
    let config = RelayConfig::load_from_env()?;
    let scoped_verifier = scoped_verifier_from_env()?;
    if let Some(message) = open_access_startup_error(
        &config.host,
        config.shared_token.is_some(),
        scoped_verifier.is_some(),
        parse_env_flag(
            std::env::var("CHARIOX_RELAY_ALLOW_OPEN_ACCESS")
                .ok()
                .as_deref(),
        ),
    ) {
        eprintln!(
            "{}",
            json!({
                "component": "chariox-relay",
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
    let draining = parse_relay_draining(std::env::var("CHARIOX_RELAY_DRAINING").ok().as_deref());
    server.set_draining(draining);
    eprintln!(
        "{}",
        json!({
            "component": "chariox-relay",
            "level": "info",
            "event": "relay_process_starting",
            "fields": {
                "host": config.host,
                "port": config.port,
                "package_version": env!("CARGO_PKG_VERSION"),
                "build_commit": std::env::var("CHARIOX_BUILD_COMMIT").ok(),
                "draining": draining,
                "scoped_verifier": std::env::var("CHARIOX_RELAY_SCOPED_ISSUER")
                    .ok()
                    .map(|value| !value.trim().is_empty())
                    .unwrap_or(false),
            },
        })
    );
    // Optional: pull account/identity revocations from the hosted control
    // plane into this relay's registry so revoked tokens are rejected.
    let _revocation_shutdown_tx =
        if let Some((cloud_url, realm, secret)) = revocation_sync_from_env()? {
            let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
            eprintln!(
                "{}",
                json!({
                    "component": "chariox-relay",
                    "level": "info",
                    "event": "revocation_sync_enabled",
                    "fields": { "realm_id": realm },
                })
            );
            tokio::spawn(chariox_relay::revocation_sync::run_revocation_sync(
                cloud_url,
                realm,
                secret,
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

fn revocation_sync_from_env() -> Result<Option<(String, String, String)>, String> {
    let read = |name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let cloud_url = read("CHARIOX_RELAY_REVOCATION_URL");
    let realm = read("CHARIOX_RELAY_REVOCATION_REALM");
    if cloud_url.is_none() && realm.is_none() {
        return Ok(None);
    }
    let cloud_url = cloud_url.ok_or("revocation sync requires CHARIOX_RELAY_REVOCATION_URL")?;
    let realm = realm.ok_or("revocation sync requires CHARIOX_RELAY_REVOCATION_REALM")?;
    let secret = read("CHARIOX_RELAY_SCOPED_HMAC_SECRET")
        .ok_or("authenticated Cloud revocation sync requires CHARIOX_RELAY_SCOPED_HMAC_SECRET")?;
    Ok(Some((cloud_url, realm, secret)))
}

fn revocation_sync_interval_from_env() -> std::time::Duration {
    std::env::var("CHARIOX_RELAY_REVOCATION_INTERVAL_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map(std::time::Duration::from_secs)
        .unwrap_or(chariox_relay::revocation_sync::DEFAULT_REVOCATION_SYNC_INTERVAL)
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
         set CHARIOX_RELAY_TOKEN, configure a scoped issuer, or explicitly opt in \
         with CHARIOX_RELAY_ALLOW_OPEN_ACCESS=1"
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

fn scoped_verifier_from_env() -> Result<Option<RelayAuthVerifier>, String> {
    scoped_verifier_from_settings(
        std::env::var("CHARIOX_RELAY_SCOPED_ISSUER").ok(),
        std::env::var("CHARIOX_RELAY_SCOPED_HMAC_SECRET").ok(),
    )
}

fn scoped_verifier_from_settings(
    issuer: Option<String>,
    secret: Option<String>,
) -> Result<Option<RelayAuthVerifier>, String> {
    if issuer.is_none() && secret.is_none() {
        return Ok(None);
    }
    let issuer = issuer
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "scoped relay configuration requires a nonblank issuer".to_string())?;
    let secret = secret
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "scoped relay configuration requires a nonblank HMAC secret".to_string())?;
    Ok(Some(RelayAuthVerifier::scoped_hmac(
        BTreeMap::from([(issuer, secret)]),
        None,
    )))
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
    #[test]
    fn partial_or_blank_scoped_configuration_fails_closed() {
        assert!(scoped_verifier_from_settings(None, None).unwrap().is_none());
        assert!(scoped_verifier_from_settings(
            Some("issuer".into()),
            Some("synthetic-secret".into())
        )
        .unwrap()
        .is_some());
        for (issuer, secret) in [
            (Some("issuer"), None),
            (None, Some("synthetic-secret")),
            (Some(""), None),
            (None, Some(" ")),
            (Some("issuer"), Some(" ")),
            (Some(" "), Some("synthetic-secret")),
        ] {
            assert!(scoped_verifier_from_settings(
                issuer.map(str::to_string),
                secret.map(str::to_string)
            )
            .is_err());
        }
    }
}
