//! MP-08/MP-10/MP-11: mirror wiring keeps terminal, Vault and input authority below clients.
use super::kernel_browser_runtime::host_error;
use super::KernelRuntimeState;
use crate::error::DaemonError;
use crate::local::KernelBrowserCommand;
use crate::runtime::command::KernelCommand;
use crate::transport::room_shared_display::RoomDisplayAudience;
use serde_json::{json, Value};

impl KernelRuntimeState {
    pub(super) async fn kernel_browser_mirror_request(
        &self,
        caller: &KernelCommand,
        command: KernelBrowserCommand,
    ) -> Result<Value, DaemonError> {
        if !crate::runtime::kernel_browser_features::mirror_enabled() {
            return Err(host_error("MP-08: DOM mirroring disabled".into()));
        }
        let (user, actor) = self.kernel_browser_terminal_context(caller)?;
        let input = matches!(command, KernelBrowserCommand::MirrorInput { .. });
        let params = mirror_host_params(command, &actor, &caller.command_id)?;
        let admission = self
            .owned
            .kernel_browser_host
            .admit_terminal(&user, caller.terminal_lifetime.clone().unwrap_or_default());
        let result = self
            .kernel_browser_operation_admitted(&user, Some(admission), "host.browser", params)
            .await?;
        Ok(mirror_reply(input, result))
    }
}

/// The host request for a terminal's mirror command. Only the kernel adds the
/// audience; typed client commands cannot carry one.
fn mirror_host_params(
    command: KernelBrowserCommand,
    actor: &str,
    command_id: &str,
) -> Result<Value, DaemonError> {
    let subscribe = matches!(command, KernelBrowserCommand::MirrorSubscribe { .. });
    let next = matches!(command, KernelBrowserCommand::MirrorNext { .. });
    let mut params = match command {
        KernelBrowserCommand::MirrorInput {
            tab_id,
            generation,
            document_id,
            subscription_id,
            sequence,
            action,
        } => {
            if document_id.is_empty() || document_id.len() > 256 || subscription_id.len() > 256 {
                return Err(host_error("MP-11: invalid mirror document binding".into()));
            }
            let action = serde_json::to_value(action)
                .map_err(|_| host_error("MP-11: invalid mirror action".into()))?;
            if ["text", "value"]
                .iter()
                .any(|name| action[name].as_str().is_some_and(|text| text.len() > 16384))
            {
                return Err(host_error("MP-11: mirror input text exceeds bound".into()));
            }
            // The host and shared actor ledger see ordinary input. No second
            // permission, cancellation, takeover, held-key or history path.
            json!({"op":"input","tab_id":tab_id,"generation":generation,"document_id":document_id,"input":{"kind":"mirror","subscription_id":subscription_id,"sequence":sequence,"action":action}})
        }
        command @ (KernelBrowserCommand::MirrorSubscribe { .. }
        | KernelBrowserCommand::MirrorNext { .. }
        | KernelBrowserCommand::MirrorClose { .. }) => serde_json::to_value(command)
            .map_err(|_| host_error("MP-11: invalid mirror command".into()))?,
        _ => return Err(host_error("MP-11: mirror command required".into())),
    };
    params["observed_by"] = json!(actor);
    if subscribe {
        // VAUDIT S3 / AU-03: a user-domain browser belongs to its user, so an
        // authenticated terminal of that user is its owner and sees Vault
        // fills and captures unmasked, as on the desktop's owner feed. Only
        // the kernel names the audience; model observations stay masked.
        params["audience"] = json!(RoomDisplayAudience::Owner);
    }
    if next {
        // A credit replayed after a reconnect keeps its command id: the
        // controller answers it with the original packet (no new sequence).
        params["command_id"] = json!(command_id);
    }
    Ok(params)
}

/// MP-08/MP-10: protocol 489 answers an admitted mirror input with an
/// acknowledgement only. The full browser state already reconciled the actor
/// ledger below; the viewer sees the input's effect in the next packet. v2
/// packets arrive already encoded by the controller (compact deltas, bodies
/// deflated in the subscription's context) and pass through unchanged.
fn mirror_reply(input: bool, result: Value) -> Value {
    if input {
        json!({"accepted": true})
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_input_reply_is_an_acknowledgement_and_packets_pass_through() {
        // Protocol 482 answered every keystroke with the full browser state
        // (~0.77 KB on the hosted wire); 489 acknowledges only.
        let state = json!({"state":"running","generation":3,"tabs":[{"tab_id":"t","url":"https://www.wikipedia.org/","title":"Wikipedia"}]});
        assert_eq!(mirror_reply(true, state), json!({"accepted": true}));
        let packet = json!({"wire":2,"sequence":4,"encoding":"deflate","packet_bytes":900,"packet_base64":"AAAA"});
        assert_eq!(mirror_reply(false, packet.clone()), packet);
    }

    // VAUDIT S3 / AU-03: the owner audience is the kernel's to name, on subscriptions only.
    #[test]
    fn only_the_kernel_names_the_owner_audience_of_a_mirror_subscription() {
        let subscribe: KernelBrowserCommand = serde_json::from_value(
            json!({"op":"mirror_subscribe","tab_id":"t","generation":1,"device_scale_factor":1,"wire":2}),
        )
        .unwrap();
        let params = mirror_host_params(subscribe, "terminal:owner", "c1").unwrap();
        assert_eq!(params["audience"], "owner");
        assert_eq!(params["observed_by"], "terminal:owner");
        let next: KernelBrowserCommand = serde_json::from_value(
            json!({"op":"mirror_next","subscription_id":"s","generation":1,"after_sequence":0}),
        )
        .unwrap();
        assert!(mirror_host_params(next, "terminal:owner", "c2")
            .unwrap()
            .get("audience")
            .is_none());
        for audience in ["owner", "viewer"] {
            assert!(
                serde_json::from_value::<KernelBrowserCommand>(json!({"op":"mirror_subscribe","tab_id":"t","generation":1,"device_scale_factor":1,"audience":audience})).is_err(),
                "a client cannot choose the {audience} audience"
            );
        }
    }
}
