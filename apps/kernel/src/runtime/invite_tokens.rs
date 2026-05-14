use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::error::DaemonError;
use crate::local::PairingInviteIntent;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SessionInviteToken {
    pub(crate) version: u8,
    pub(crate) session_id: String,
    pub(crate) invite_id: String,
    pub(crate) created_by_user_id: String,
    pub(crate) issued_at_ms: u64,
    #[serde(default)]
    pub(crate) expires_at_ms: Option<u64>,
    #[serde(default)]
    pub(crate) max_uses: Option<u32>,
}

pub(crate) fn encode_session_invite_token(
    token: &SessionInviteToken,
) -> Result<String, DaemonError> {
    let payload = serde_json::to_vec(token).map_err(|error| DaemonError::LocalTransport {
        operation: "encode session invite",
        message: error.to_string(),
    })?;
    Ok(format!(
        "arroba-session-invite-v1.{}",
        URL_SAFE_NO_PAD.encode(payload)
    ))
}

pub(crate) fn decode_session_invite_token(token: &str) -> Result<SessionInviteToken, DaemonError> {
    let payload = token
        .trim()
        .strip_prefix("arroba-session-invite-v1.")
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "decode session invite",
            message: "session invite token has an unsupported format".to_string(),
        })?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "decode session invite",
            message: error.to_string(),
        })?;
    let decoded = serde_json::from_slice::<SessionInviteToken>(&bytes).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "decode session invite",
            message: error.to_string(),
        }
    })?;
    if decoded.version != 1 {
        return Err(DaemonError::LocalTransport {
            operation: "decode session invite",
            message: format!("unsupported session invite version {}", decoded.version),
        });
    }
    Ok(decoded)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PairingInviteToken {
    pub(crate) version: u8,
    pub(crate) intent: PairingInviteIntent,
    pub(crate) invite_id: String,
    pub(crate) relay_url: String,
    pub(crate) relay_token: String,
    pub(crate) target_daemon_id: String,
    #[serde(default)]
    pub(crate) target_daemon_alias: Option<String>,
    pub(crate) issuer_machine_id: String,
    pub(crate) issued_at_ms: u64,
    pub(crate) expires_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) terminal_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pairing_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) terminal_id: Option<String>,
}

pub(crate) fn encode_pairing_invite_token(
    token: &PairingInviteToken,
) -> Result<String, DaemonError> {
    encode_pairing_invite_token_with_prefix("arroba-invite-v1", token)
}

pub(crate) fn encode_terminal_pairing_link(
    token: &PairingInviteToken,
) -> Result<String, DaemonError> {
    encode_pairing_invite_token_with_prefix("arroba-terminal-pair-v1", token)
}

fn encode_pairing_invite_token_with_prefix(
    prefix: &str,
    token: &PairingInviteToken,
) -> Result<String, DaemonError> {
    let payload = serde_json::to_vec(token).map_err(|error| DaemonError::LocalTransport {
        operation: "encode pairing invite",
        message: error.to_string(),
    })?;
    Ok(format!("{prefix}.{}", URL_SAFE_NO_PAD.encode(payload)))
}

pub(crate) fn decode_pairing_invite_token(token: &str) -> Result<PairingInviteToken, DaemonError> {
    let trimmed = token.trim();
    let payload = trimmed
        .strip_prefix("arroba-invite-v1.")
        .or_else(|| trimmed.strip_prefix("arroba-terminal-pair-v1."))
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "decode pairing invite",
            message: "pairing invite token has an unsupported format".to_string(),
        })?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "decode pairing invite",
            message: error.to_string(),
        })?;
    let decoded = serde_json::from_slice::<PairingInviteToken>(&bytes).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "decode pairing invite",
            message: error.to_string(),
        }
    })?;
    if decoded.version != 1 {
        return Err(DaemonError::LocalTransport {
            operation: "decode pairing invite",
            message: format!("unsupported pairing invite version {}", decoded.version),
        });
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_invite_tokens_round_trip() {
        let token = SessionInviteToken {
            version: 1,
            session_id: "session-1".to_string(),
            invite_id: "invite-1".to_string(),
            created_by_user_id: "user-1".to_string(),
            issued_at_ms: 100,
            expires_at_ms: Some(200),
            max_uses: Some(1),
        };

        let encoded = encode_session_invite_token(&token).unwrap();
        let decoded = decode_session_invite_token(&encoded).unwrap();

        assert_eq!(decoded.session_id, "session-1");
        assert_eq!(decoded.invite_id, "invite-1");
        assert_eq!(decoded.expires_at_ms, Some(200));
    }

    #[test]
    fn pairing_tokens_accept_invite_and_terminal_link_prefixes() {
        let token = PairingInviteToken {
            version: 1,
            intent: PairingInviteIntent::Client,
            invite_id: "invite-1".to_string(),
            relay_url: "ws://relay".to_string(),
            relay_token: "relay-token".to_string(),
            target_daemon_id: "daemon-1".to_string(),
            target_daemon_alias: Some("main".to_string()),
            issuer_machine_id: "machine-1".to_string(),
            issued_at_ms: 100,
            expires_at_ms: 200,
            terminal_type: Some("cli".to_string()),
            pairing_code: Some("ABCD-1234".to_string()),
            terminal_id: Some("cli-1".to_string()),
        };

        let invite =
            decode_pairing_invite_token(&encode_pairing_invite_token(&token).unwrap()).unwrap();
        let terminal =
            decode_pairing_invite_token(&encode_terminal_pairing_link(&token).unwrap()).unwrap();

        assert_eq!(invite.invite_id, "invite-1");
        assert_eq!(terminal.terminal_id.as_deref(), Some("cli-1"));
    }

    #[test]
    fn invite_tokens_reject_unknown_prefixes() {
        assert!(decode_session_invite_token("bad.token").is_err());
        assert!(decode_pairing_invite_token("bad.token").is_err());
    }
}
