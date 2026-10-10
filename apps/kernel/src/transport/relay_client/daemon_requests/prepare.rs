//! Validated decryption before asynchronous request dispatch; no payload logging.
use super::*;

pub(in crate::transport::relay_client) struct PreparedDaemonRequest {
    pub(super) message: ParsedRelayClientMessage,
    pub(super) client_public_key: String,
    pub(super) daemon_private_key: String,
    pub(super) caller_identity: Option<RelayCallerIdentity>,
    pub(super) encrypted_request: EncryptedRelayPayload,
}
impl PreparedDaemonRequest {
    pub(in crate::transport::relay_client) fn browser_input_key(&self) -> Option<String> {
        use crate::local::{KernelBrowserCommand as C, KernelBrowserRequest};
        let ParsedRelayClientMessage::Request(request) = &self.message else {
            return None;
        };
        let LocalDaemonRequest::KernelBrowser(KernelBrowserRequest { command }) = &request.request
        else {
            return None;
        };
        // MP-08/MP-10/MP-11: native human chords and committed text use
        // the same socket-order reservation; state/credits remain concurrent.
        // A distinct domain and live native generation prevent key collisions.
        if let C::Computer {
            command: crate::local::KernelComputerCommand::Input { target, .. },
        } = command
        {
            if target.surface_id.is_empty()
                || target.surface_id.len() > 256
                || target.generation.is_empty()
                || target.generation.len() > 256
            {
                return None;
            }
            let caller = self.caller_identity.as_ref()?;
            return serde_json::to_string(&(
                &caller.realm_id,
                &caller.subject,
                &caller.user_id,
                "desktop",
                &target.surface_id,
                &target.generation,
            ))
            .ok();
        }
        let (tab, generation) = match command {
            C::Input {
                tab_id, generation, ..
            }
            | C::DisplayInput {
                tab_id, generation, ..
            }
            | C::MirrorInput {
                tab_id, generation, ..
            } => (tab_id, generation),
            _ => return None,
        };
        if tab.is_empty() || tab.len() > 256 {
            return None;
        }
        let caller = self.caller_identity.as_ref()?;
        // Browser requests use ephemeral encryption keys. Their authenticated
        // caller/tab binding, not that ephemeral key, owns scheduling order.
        serde_json::to_string(&(
            &caller.realm_id,
            &caller.subject,
            &caller.user_id,
            tab,
            generation,
        ))
        .ok()
    }
}

pub(in crate::transport::relay_client) fn prepare_daemon_request(
    router: &CommandRouter,
    caller_identity: Option<RelayCallerIdentity>,
    encrypted_request: EncryptedRelayPayload,
) -> Result<PreparedDaemonRequest, RelayRequestOutcome> {
    let refusal = |error| RelayRequestOutcome {
        display_event: None,
        encrypted_response: None,
        error: Some(error),
    };
    relay_crypto::validate_encrypted_payload_shape(
        &encrypted_request,
        MAX_BROWSER_IMPORT_ENCRYPTED_BYTES,
    )
    .map_err(|_| {
        refusal(relay_error(
            "invalid_request",
            "invalid relay request payload",
            false,
        ))
    })?;
    validate_bound_service_sender(caller_identity.as_ref(), &encrypted_request).map_err(refusal)?;
    let daemon_private_key = router.relay_private_key();
    let decrypted =
        relay_crypto::decrypt_payload_for_private_key(&daemon_private_key, &encrypted_request)
            .map_err(|error| {
                refusal(relay_error(
                    "invalid_request",
                    &format!("invalid relay request payload: {error}"),
                    false,
                ))
            })?;
    let message = parse_relay_client_request(&decrypted.plaintext).map_err(|error| {
        refusal(relay_error(
            "invalid_request",
            &crate::transport::request_decode_error::message(
                "invalid relay request payload",
                &error,
            ),
            false,
        ))
    })?;
    Ok(PreparedDaemonRequest {
        message,
        client_public_key: decrypted.sender_public_key,
        daemon_private_key,
        caller_identity,
        encrypted_request,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chariox_relay::auth::RelaySubjectKind;
    fn prepared(command: crate::local::KernelBrowserCommand) -> PreparedDaemonRequest {
        PreparedDaemonRequest {
            message: ParsedRelayClientMessage::Request(ParsedRelayClientRequest {
                command_id: None,
                request: LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest {
                    command,
                }),
            }),
            client_public_key: "ephemeral-first".into(),
            daemon_private_key: String::new(),
            encrypted_request: EncryptedRelayPayload {
                sender_public_key: "ephemeral-first".into(),
                nonce: String::new(),
                ciphertext: String::new(),
            },
            caller_identity: Some(RelayCallerIdentity {
                realm_id: "realm".into(),
                subject: "terminal".into(),
                subject_kind: RelaySubjectKind::Client,
                expires_at_ms: 0,
                token_id: None,
                user_id: Some("owner".into()),
                public_key_thumbprint: None,
            }),
        }
    }
    #[test]
    fn mp08_ephemeral_sender_changes_do_not_split_browser_input_order() {
        use crate::local::KernelBrowserCommand as C;
        let mut first = prepared(C::DisplayInput {
            tab_id: "tab".into(),
            generation: 1,
            document_id: "doc".into(),
            input: crate::local::KernelBrowserInput::Text {
                text: "public".into(),
            },
        });
        let key = first.browser_input_key().unwrap();
        first.client_public_key = "ephemeral-second".into();
        first.encrypted_request.sender_public_key = first.client_public_key.clone();
        assert_eq!(first.browser_input_key().unwrap(), key);
        first.caller_identity.as_mut().unwrap().user_id = Some("other".into());
        assert_ne!(first.browser_input_key().unwrap(), key);
    }
    #[test]
    fn mp08_display_credits_and_unidentified_requests_do_not_take_input_turns() {
        use crate::local::KernelBrowserCommand as C;
        assert!(prepared(C::DisplayAck {
            subscription_id: "s".into(),
            generation: 1,
            sequence: 0,
            lost: false
        })
        .browser_input_key()
        .is_none());
        assert!(prepared(C::State).browser_input_key().is_none());
        let mut input = prepared(C::DisplayInput {
            tab_id: "tab".into(),
            generation: 1,
            document_id: "doc".into(),
            input: crate::local::KernelBrowserInput::Text {
                text: "public".into(),
            },
        });
        input.caller_identity = None;
        assert!(input.browser_input_key().is_none());
    }

    // MP-08/MP-10/MP-11: Computer shares the authenticated KernelBrowser ingress.
    fn computer_input(surface: &str, generation: &str) -> PreparedDaemonRequest {
        prepared(
            serde_json::from_value(serde_json::json!({
                "op":"computer", "command":{"op":"input",
                    "target":{"surface_id":surface,"generation":generation},
                    "input":{"kind":"key","key":"Control+a"}}
            }))
            .unwrap(),
        )
    }
    #[test]
    fn mp08_computer_input_reserves_authenticated_surface_order() {
        let mut first = computer_input("desktop", "generation");
        let key = first
            .browser_input_key()
            .expect("MP-08/MP-10/MP-11: Computer input missing ingress order");
        first.client_public_key = "another-ephemeral-key".into();
        assert_eq!(first.browser_input_key().unwrap(), key);
        first.caller_identity.as_mut().unwrap().user_id = Some("another-owner".into());
        assert_ne!(first.browser_input_key().unwrap(), key);
        first.caller_identity = None;
        assert!(first.browser_input_key().is_none());
    }
    #[test]
    fn mp08_computer_surface_generations_have_independent_order() {
        let first = computer_input("desktop", "first")
            .browser_input_key()
            .unwrap();
        assert_ne!(
            computer_input("desktop", "second")
                .browser_input_key()
                .unwrap(),
            first
        );
        assert_ne!(
            computer_input("another-desktop", "first")
                .browser_input_key()
                .unwrap(),
            first
        );
    }
    #[test]
    fn mp08_computer_state_does_not_take_an_input_turn() {
        assert!(prepared(
            serde_json::from_value(serde_json::json!({
                "op":"computer", "command":{"op":"state"}
            }))
            .unwrap()
        )
        .browser_input_key()
        .is_none());
    }
}
