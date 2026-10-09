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
        mirror_wire_result(next, result)
    }
}

/// Only v2 packets (`mirror_next` results) are compressed; subscribe/close
/// replies keep their plain fields (`subscription_id`, `wire`). A small delta
/// (an echo) travels plain: inflating it would only add a client task hop.
fn mirror_wire_result(next: bool, result: Value) -> Result<Value, DaemonError> {
    if next
        && result.get("wire").and_then(Value::as_u64) == Some(2)
        && serde_json::to_vec(&result).map_or(true, |body| body.len() >= 2048)
    {
        return compress_mirror_packet(result);
    }
    Ok(result)
}

/// MP-08/MP-10: a protocol 482 packet travels gzip-compressed after the Vault
/// scrub (the relay has no permessage-deflate). Already-encoded resource and
/// region bytes stay outside the compressed body.
fn compress_mirror_packet(mut packet: Value) -> Result<Value, DaemonError> {
    use base64::Engine as _;
    use std::io::Write as _;
    let object = packet
        .as_object_mut()
        .ok_or_else(|| host_error("MP-11: invalid mirror packet".into()))?;
    let resources = object.remove("resources").unwrap_or_else(|| json!([]));
    let tiles = object.remove("tiles").unwrap_or_else(|| json!([]));
    let body = serde_json::to_vec(&packet)
        .map_err(|_| host_error("MP-11: invalid mirror packet".into()))?;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    encoder
        .write_all(&body)
        .map_err(|_| host_error("MP-10: mirror packet compression failed".into()))?;
    let compressed = encoder
        .finish()
        .map_err(|_| host_error("MP-10: mirror packet compression failed".into()))?;
    Ok(json!({
        "wire": 2,
        "encoding": "gzip",
        "packet_bytes": body.len(),
        "packet_base64": base64::engine::general_purpose::STANDARD.encode(compressed),
        "resources": resources,
        "tiles": tiles,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    #[test]
    fn mirror_v2_subscribe_reply_stays_plain_and_only_packets_compress() {
        // The first hosted run sent the subscribe reply gzipped: clients lost
        // `subscription_id` and every later mirror request failed to decode.
        let subscribed = json!({"subscription_id":"host-mirror-1","generation":1,"tab_id":"t","device_scale_factor":1,"wire":2});
        assert_eq!(mirror_wire_result(false, subscribed.clone()).unwrap(), subscribed);
        let packet = json!({"wire":2,"sequence":1,"ops":[{"op":"text","id":"n2","text":"a".repeat(4096)}],"resources":[],"tiles":[]});
        assert_eq!(mirror_wire_result(true, packet).unwrap()["encoding"], "gzip");
        let echo = json!({"wire":2,"sequence":2,"ops":[{"op":"form","id":"n9"}],"resources":[],"tiles":[]});
        assert_eq!(mirror_wire_result(true, echo.clone()).unwrap(), echo);
        let v1 = json!({"sequence":1,"nodes":[]});
        assert_eq!(mirror_wire_result(true, v1.clone()).unwrap(), v1);
    }

    #[test]
    fn mirror_v2_packet_compresses_body_and_keeps_media_outside() {
        let packet = json!({"wire":2,"sequence":4,"ops":[{"op":"text","id":"n2","text":"a".repeat(4096)}],
            "resources":[{"key":"r1","data_base64":"AAAA"}],"tiles":[]});
        let wire = compress_mirror_packet(packet.clone()).unwrap();
        assert_eq!(wire["encoding"], "gzip");
        assert_eq!(wire["resources"], packet["resources"]);
        use base64::Engine as _;
        let compressed = base64::engine::general_purpose::STANDARD
            .decode(wire["packet_base64"].as_str().unwrap())
            .unwrap();
        assert!(compressed.len() < 512, "MP-10: DOM text compresses");
        let mut body = String::new();
        flate2::read::GzDecoder::new(&compressed[..])
            .read_to_string(&mut body)
            .unwrap();
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["ops"], packet["ops"]);
        assert!(body.get("resources").is_none());
        assert_eq!(wire["packet_bytes"].as_u64().unwrap() as usize, serde_json::to_vec(&body).unwrap().len());
    }
}
