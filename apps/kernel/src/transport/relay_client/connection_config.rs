//! Relay socket continuity decisions for config/token changes.

#[derive(Debug, PartialEq, Eq)]
pub(super) enum RelayConfigContinuity {
    Continue,
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
        Some(_) => RelayConfigContinuity::Reconnect("relay token changed"),
        None => RelayConfigContinuity::Reconnect("relay token missing"),
    }
}

#[cfg(test)]
mod tests {
    use super::{relay_config_continuity, RelayConfigContinuity};

    use crate::config::DaemonConfig;

    #[test]
    fn relay_config_continuity_reconnects_for_token_rotation() {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("wss://relay.example/ws".to_string());
        config.relay_token = Some("new-token".to_string());

        assert_eq!(
            relay_config_continuity("wss://relay.example/ws", "old-token", &config),
            RelayConfigContinuity::Reconnect("relay token changed")
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
}
