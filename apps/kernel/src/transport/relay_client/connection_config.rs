//! Relay socket continuity decisions for config/token changes.

use base64::Engine;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum RelayConfigContinuity {
    Continue,
    TokenRotated(String),
    Reconnect(&'static str),
}

pub(super) fn relay_config_continuity(
    active_relay_url: &str,
    active_relay_token: &str,
    config: &crate::config::DaemonConfig,
) -> RelayConfigContinuity {
    if config.relay_url.as_deref() != Some(active_relay_url) || config.relay_token.is_none() {
        return RelayConfigContinuity::Reconnect("relay url or token missing changed");
    }
    match config.relay_token.as_deref() {
        Some(token) if token == active_relay_token => RelayConfigContinuity::Continue,
        Some(token) if relay_token_realm_changed(active_relay_token, token) => {
            RelayConfigContinuity::Reconnect("relay token realm changed")
        }
        Some(token) => RelayConfigContinuity::TokenRotated(token.to_string()),
        None => RelayConfigContinuity::Reconnect("relay token missing"),
    }
}

fn relay_token_realm_changed(active_token: &str, next_token: &str) -> bool {
    match (
        unverified_relay_token_realm(active_token),
        unverified_relay_token_realm(next_token),
    ) {
        (Some(active), Some(next)) => active != next,
        _ => false,
    }
}

fn unverified_relay_token_realm(token: &str) -> Option<String> {
    let payload = token.trim().split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice::<serde_json::Value>(&decoded)
        .ok()?
        .get("realm_id")?
        .as_str()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{relay_config_continuity, RelayConfigContinuity};

    use base64::Engine;

    use crate::config::DaemonConfig;

    #[test]
    fn relay_config_continuity_keeps_active_socket_for_token_rotation() {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example/ws".to_string());
        config.relay_token = Some(token_for_realm("realm-1", "new").to_string());

        assert_eq!(
            relay_config_continuity(
                "wss://relay.example/ws",
                &token_for_realm("realm-1", "old"),
                &config,
            ),
            RelayConfigContinuity::TokenRotated(token_for_realm("realm-1", "new"))
        );
    }

    #[test]
    fn relay_config_continuity_reconnects_when_rotated_token_changes_realm() {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example/ws".to_string());
        config.relay_token = Some(token_for_realm("new-realm", "new"));

        assert_eq!(
            relay_config_continuity(
                "wss://relay.example/ws",
                &token_for_realm("old-realm", "old"),
                &config,
            ),
            RelayConfigContinuity::Reconnect("relay token realm changed")
        );
    }

    #[test]
    fn relay_config_continuity_reconnects_for_url_changes_or_disabled_relay() {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example/ws".to_string());
        config.relay_token = Some("token".to_string());
        assert!(matches!(
            relay_config_continuity("wss://other.example/ws", "token", &config),
            RelayConfigContinuity::Reconnect(_)
        ));

        config.relay_token = None;
        assert!(matches!(
            relay_config_continuity("wss://relay.example/ws", "token", &config),
            RelayConfigContinuity::Reconnect(_)
        ));
    }

    fn token_for_realm(realm_id: &str, signature: &str) -> String {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::json!({ "realm_id": realm_id }).to_string());
        format!("header.{payload}.{signature}")
    }
}
