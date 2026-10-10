//! MP-08/MP-10/MP-11: mirror wiring keeps terminal, Vault and input authority below clients.
use super::kernel_browser_runtime::host_error;
use super::KernelRuntimeState;
use crate::error::DaemonError;
use crate::local::KernelBrowserCommand;
use crate::runtime::command::KernelCommand;
use serde_json::{json, Value};

impl KernelRuntimeState {
    pub(super) async fn kernel_browser_mirror_request(
        &self,
        caller: &KernelCommand,
        command: KernelBrowserCommand,
    ) -> Result<Value, DaemonError> {
        if std::env::var("CHARIOX_KERNEL_BROWSER_MIRROR").as_deref() != Ok("1") {
            return Err(host_error("MP-08: DOM mirroring disabled".into()));
        }
        let (user, actor) = self.kernel_browser_terminal_context(caller)?;
        let next = matches!(command, KernelBrowserCommand::MirrorNext { .. });
        let input = matches!(command, KernelBrowserCommand::MirrorInput { .. });
        let mut params = match command {
            KernelBrowserCommand::MirrorInput {
                tab_id,
                generation,
                document_id,
                subscription_id,
                sequence,
                action,
            } => {
                if document_id.is_empty() || document_id.len() > 256 || subscription_id.len() > 256
                {
                    return Err(host_error("MP-11: invalid mirror document binding".into()));
                }
                let action = serde_json::to_value(action)
                    .map_err(|_| host_error("MP-11: invalid mirror action".into()))?;
                if action["text"]
                    .as_str()
                    .is_some_and(|text| text.len() > 16384)
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
        if next {
            // A credit replayed after a reconnect keeps its command id: the
            // controller answers it with the original packet (no new sequence).
            params["command_id"] = json!(caller.command_id);
        }
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
}
