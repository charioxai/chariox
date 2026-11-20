//! A Vault entry typed into a provider's own API-key login. The kernel reads
//! the value, waits for the login to ask for it and types it; neither the
//! request nor its answer carries the value, and the login screen stays masked.
use super::provider_auth_control::{
    execute_get_provider_login_status_request, provider_login_error, serialized_login_record,
};
use crate::error::DaemonError;
use crate::local::{
    GetProviderLoginStatusRequest, LocalDaemonResponse, ProviderLoginProcessState,
    SendProviderLoginVaultEntryRequest,
};
use crate::runtime::command::KernelCommand;
use crate::runtime::state::{KernelRuntimeState, ProviderLoginProcessBackend};
use base64::Engine as _;
use zeroize::Zeroizing;

/// A login that never shows its key prompt is stuck; the caller can cancel it.
const KEY_PROMPT_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
const KEY_PROMPT_POLL: std::time::Duration = std::time::Duration::from_millis(150);

pub(super) async fn execute_send_provider_login_vault_entry_request(
    runtime_state: &KernelRuntimeState,
    command: &KernelCommand,
    owner_user_id: &str,
    request: SendProviderLoginVaultEntryRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let origin = {
        let (_serial, record) =
            serialized_login_record(runtime_state, owner_user_id, &request.login_id).await?;
        accepting_origin(&record)?
    };
    let value = runtime_state
        .read_provider_login_vault_value(command, &request.credential_id, origin)
        .await?;
    wait_for_key_prompt(runtime_state, owner_user_id, &request.login_id).await?;
    let (_serial, record) =
        serialized_login_record(runtime_state, owner_user_id, &request.login_id).await?;
    accepting_origin(&record)?;
    let mut typed = Zeroizing::new(value.as_bytes().to_vec());
    typed.push(b'\r');
    let store = runtime_state.provider_login_process_store();
    // Masked before it is written, so no echo can reach the record unmasked.
    store.vault_value_typed(
        owner_user_id,
        &request.login_id,
        value,
        crate::session::unix_epoch_ms(),
    )?;
    runtime_state
        .with_app_side_effect(|app| app.pty_mut().write_input(&request.login_id, &typed))
        .await?;
    Ok(LocalDaemonResponse::ProviderLoginVaultEntrySent {
        login_id: request.login_id,
    })
}

/// The origin whose Vault entry this login takes, while it can still take one.
fn accepting_origin(
    record: &crate::runtime::state::ProviderLoginProcessRecord,
) -> Result<&'static str, DaemonError> {
    if record.state != ProviderLoginProcessState::Running {
        return Err(provider_login_error("provider login is not running"));
    }
    if record.backend != ProviderLoginProcessBackend::Terminal {
        return Err(provider_login_error(
            "this provider login does not accept terminal input",
        ));
    }
    let origin = record.vault_origin.ok_or_else(|| {
        provider_login_error(
            "this sign-in does not take an API key; use a method that does, such as an API key sign-in",
        )
    })?;
    if record.typed_secret.is_some() {
        return Err(provider_login_error(
            "a Vault entry was already typed into this sign-in; cancel it and start again to use another",
        ));
    }
    Ok(origin)
}

async fn wait_for_key_prompt(
    runtime_state: &KernelRuntimeState,
    owner_user_id: &str,
    login_id: &str,
) -> Result<(), DaemonError> {
    let deadline = tokio::time::Instant::now() + KEY_PROMPT_WAIT;
    loop {
        let LocalDaemonResponse::ProviderLoginStatus { login } =
            execute_get_provider_login_status_request(
                runtime_state,
                owner_user_id,
                GetProviderLoginStatusRequest {
                    login_id: login_id.to_string(),
                },
            )
            .await?
        else {
            return Err(provider_login_error("provider login status is unavailable"));
        };
        if login.state != ProviderLoginProcessState::Running {
            return Err(provider_login_error(
                "the sign-in ended before it asked for the API key; start it again",
            ));
        }
        let screen = base64::engine::general_purpose::STANDARD
            .decode(&login.terminal_output_base64)
            .unwrap_or_default();
        if asks_for_api_key(&screen) {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(provider_login_error(
                "the sign-in did not ask for the API key in time; cancel it and start again",
            ));
        }
        tokio::time::sleep(KEY_PROMPT_POLL).await;
    }
}

/// opencode's password prompt: "Enter your API key". Colour codes and the
/// terminal's line breaks do not change what the words say.
fn asks_for_api_key(screen: &[u8]) -> bool {
    let mut words = String::new();
    let mut escape = false;
    for &byte in screen {
        match byte {
            0x1b => escape = true,
            b'@'..=b'~' if escape && !matches!(byte, b'[') => escape = false,
            _ if escape => {}
            _ if byte.is_ascii_whitespace() => {}
            _ => words.push(char::from(byte)),
        }
    }
    words.contains("EnteryourAPIkey")
}

#[cfg(test)]
mod tests {
    use super::asks_for_api_key;

    #[test]
    fn the_prompt_is_found_through_colour_codes_and_line_breaks() {
        assert!(asks_for_api_key(
            b"\x1b[36m\xe2\x97\x87\x1b[39m  Enter your\r\n API key\r\n"
        ));
        assert!(!asks_for_api_key(b"Select a provider\r\n"));
        assert!(!asks_for_api_key(b""));
    }
}
