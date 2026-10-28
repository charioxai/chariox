//! Validated decryption before asynchronous request dispatch; no payload logging.
use super::*;

pub(in crate::transport::relay_client) struct PreparedDaemonRequest {
    pub(super) message: ParsedRelayClientMessage,
    pub(super) client_public_key: String,
    pub(super) daemon_private_key: String,
    pub(super) caller_identity: Option<RelayCallerIdentity>,
    pub(super) encrypted_request: EncryptedRelayPayload,
    pub(super) request_memory: request_limits::RequestMemoryGuard,
}
impl PreparedDaemonRequest {
    /// MP-08/MP-10/MP-11: the order key of a viewer's browser input. `true` marks a
    /// mirror read: it observes after that viewer's earlier input and holds none of
    /// its later input, so a read sent right behind a key carries that key's echo.
    pub(in crate::transport::relay_client) fn browser_input_key(&self) -> Option<(String, bool)> {
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
            .ok()
            .map(|key| (key, false));
        }
        // A mirror subscription binds one tab and viewer; its input and reads share it.
        let (tab, generation, read) = match command {
            C::Input {
                tab_id, generation, ..
            }
            | C::DisplayInput {
                tab_id, generation, ..
            } => (tab_id, generation, false),
            C::MirrorInput {
                subscription_id,
                generation,
                ..
            } => (subscription_id, generation, false),
            C::MirrorNext {
                subscription_id,
                generation,
                ..
            } => (subscription_id, generation, true),
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
        .map(|key| (key, read))
    }
}

#[allow(
    clippy::result_large_err,
    reason = "Keep the existing typed transport error contract."
)]
pub(in crate::transport::relay_client) fn prepare_daemon_request(
    router: &CommandRouter,
    caller_identity: Option<RelayCallerIdentity>,
    encrypted_request: EncryptedRelayPayload,
) -> Result<PreparedDaemonRequest, Box<RelayRequestOutcome>> {
    let refusal = |error| {
        Box::new(RelayRequestOutcome {
            display_event: None,
            encrypted_response: None,
            error: Some(error),
        })
    };
    // The request kind is encrypted. Bound and reserve the outer bytes before
    // base64 decoding or decryption, then enforce the kind's smaller limit.
    let request_memory = request_limits::reserve(&encrypted_request).map_err(refusal)?;
    relay_crypto::validate_encrypted_payload_shape(
        &encrypted_request,
        request_limits::PROMPT_REQUEST_BYTES,
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
    request_limits::validate_kind(&message, decrypted.plaintext.len()).map_err(refusal)?;
    Ok(PreparedDaemonRequest {
        message,
        client_public_key: decrypted.sender_public_key,
        daemon_private_key,
        caller_identity,
        encrypted_request,
        request_memory,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chariox_relay::auth::RelaySubjectKind;
    fn prepared(command: crate::local::KernelBrowserCommand) -> PreparedDaemonRequest {
        let encrypted_request = EncryptedRelayPayload {
            sender_public_key: "ephemeral-first".into(),
            nonce: String::new(),
            ciphertext: String::new(),
        };
        let request_memory = request_limits::reserve(&encrypted_request).unwrap();
        PreparedDaemonRequest {
            message: ParsedRelayClientMessage::Request(ParsedRelayClientRequest {
                command_id: None,
                request: LocalDaemonRequest::KernelBrowser(crate::local::KernelBrowserRequest {
                    command,
                }),
                on_behalf_of: None,
            }),
            client_public_key: "ephemeral-first".into(),
            daemon_private_key: String::new(),
            encrypted_request,
            request_memory,
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
            binding_id: None,
            sequence: None,
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
    fn mp08_a_mirror_read_follows_its_viewers_input_without_holding_it() {
        use crate::local::{KernelBrowserCommand as C, KernelBrowserMirrorAction as A};
        let input = prepared(C::MirrorInput {
            tab_id: "tab".into(),
            generation: 1,
            document_id: "doc".into(),
            subscription_id: "mirror".into(),
            sequence: 3,
            action: A::Text {
                node_id: Some("n1".into()),
                text: "a".into(),
            },
        })
        .browser_input_key()
        .unwrap();
        let read = |subscription_id: &str| {
            prepared(C::MirrorNext {
                subscription_id: subscription_id.into(),
                generation: 1,
                after_sequence: 3,
                drift_nodes: vec![],
                known_sheets: vec![],
                wait_ms: Some(1500),
                resend: None,
                flush: None,
            })
            .browser_input_key()
            .unwrap()
        };
        assert!(!input.1);
        assert_eq!(read("mirror"), (input.0.clone(), true));
        assert_ne!(read("other").0, input.0);
    }
    /// #992 review F4 (protocol 516): through the envelope's turn rule, a viewer's
    /// read runs after its earlier key and never holds the viewer's next key.
    #[tokio::test]
    async fn mp08_a_mirror_read_enters_after_its_key_and_never_holds_the_next_key() {
        use crate::local::{KernelBrowserCommand as C, KernelBrowserMirrorAction as A};
        use crate::transport::relay_client::browser_input_order::BrowserInputOrder;
        use std::time::Duration;
        let key = |sequence| {
            prepared(C::MirrorInput {
                tab_id: "tab".into(),
                generation: 1,
                document_id: "doc".into(),
                subscription_id: "mirror".into(),
                sequence,
                action: A::Text {
                    node_id: Some("n1".into()),
                    text: "a".into(),
                },
            })
            .browser_input_key()
            .unwrap()
        };
        let read = prepared(C::MirrorNext {
            subscription_id: "mirror".into(),
            generation: 1,
            after_sequence: 3,
            drift_nodes: vec![],
            known_sheets: vec![],
            wait_ms: Some(1500),
            resend: None,
            flush: None,
        })
        .browser_input_key()
        .unwrap();
        let order = BrowserInputOrder::default();
        let (first, second) = (key(3), key(4));
        let running = order
            .reserve(first.0)
            .unwrap()
            .enter(first.1)
            .await
            .unwrap();
        let pending_read = order.reserve(read.0).unwrap();
        let next = order.reserve(second.0).unwrap();
        let reading = tokio::spawn(pending_read.enter(read.1));
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(
            !reading.is_finished(),
            "the read waits for the key before it"
        );
        drop(running);
        assert!(
            reading.await.unwrap().is_none(),
            "the read gives its turn back before it runs"
        );
        // The read's reply may still be pending; the next key does not wait for it.
        tokio::time::timeout(Duration::from_millis(50), next.enter(second.1))
            .await
            .unwrap()
            .unwrap();
    }
    #[test]
    fn mp08_display_credits_and_unidentified_requests_do_not_take_input_turns() {
        use crate::local::KernelBrowserCommand as C;
        assert!(prepared(C::DisplayAck {
            subscription_id: "s".into(),
            generation: 1,
            sequence: 0,
            lost: false,
            held_ms: None,
        })
        .browser_input_key()
        .is_none());
        assert!(prepared(C::State).browser_input_key().is_none());
        let mut input = prepared(C::DisplayInput {
            tab_id: "tab".into(),
            generation: 1,
            document_id: "doc".into(),
            binding_id: None,
            sequence: None,
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
