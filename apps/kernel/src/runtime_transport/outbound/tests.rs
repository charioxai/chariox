//! MP-11: adversarial serialized payloads generated from EVERY response/event declaration.
use super::*;
const CANARY: &str = "MP-11-outbound-test-only-secret";

fn enum_names(source: &str, declaration: &str) -> Vec<String> {
    source
        .split_once(declaration)
        .unwrap()
        .1
        .split_once("\n}")
        .unwrap()
        .0
        .lines()
        .filter_map(|line| {
            let line = line.strip_prefix("    ")?;
            let name = line.split(['{', '(', ',']).next()?.trim();
            (name.chars().next()?.is_ascii_uppercase()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            .then(|| name.to_owned())
        })
        .collect()
}
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (index, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() && index > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}
fn tainted_body() -> Value {
    serde_json::json!({"nested":[{"remote_execution":{"relay_token":CANARY,"worker_kernel_id":"worker"}}],
        "interaction":{"provider_login":{"kernel_id":"home","login":{"auth_url":CANARY,"user_code":CANARY},
            "terminal_output_base64":CANARY},"title":CANARY,"message":CANARY,"choices":[{"label":CANARY}],
            "custom_choice":{"label":CANARY},"native_origin":{"opaque":CANARY}},
        "credential":{"injection":{"kind":"header","name":"Authorization","value":CANARY}},
        "credentials":[{"injection":{"kind":"header","name":"Authorization","value":CANARY}}]})
}
#[test]
fn outbound_generated_every_response_variant_hides_worker_credentials() {
    let names = enum_names(
        include_str!("../../local/api/types/response.rs"),
        "pub enum LocalDaemonResponse {",
    );
    assert!(names.len() >= 380);
    eprintln!("MP-11 generated response variants: {}", names.len());
    let mut delivered = 0;
    for name in names {
        // Inject a remote binding at multiple depths regardless of the producing
        // handler; the final byte boundary must protect even cached raw Values.
        let body = tainted_body();
        let input = serde_json::json!({"type":"response", "request_id":"generated", "error":null,
            "response":{name.clone():body}});
        if let Ok(output) = project_payload(input) {
            delivered += 1;
            assert!(
                !output.to_string().contains(CANARY),
                "MP-11 secret escaped response {name}"
            );
        }
    }
    assert!(delivered > 300, "MP-11 public replies must remain usable");
}
#[test]
fn outbound_generated_every_event_variant_hides_worker_credentials() {
    let names = enum_names(
        include_str!("../../transport/kernel_protocol.rs"),
        "pub(crate) enum KernelEvent {",
    );
    assert!(names.len() >= 21);
    eprintln!("MP-11 generated event variants: {}", names.len());
    let mut delivered = 0;
    for name in names {
        let mut event = serde_json::json!({"event":snake(&name),
            "session":{"agents":[{"remote_execution":{"relay_token":CANARY}}]},
            "workflow_run":{"nodes":[{"remote_execution":{"relay_token":CANARY}}]}});
        event["credential"] = tainted_body()["credential"].clone();
        event["interaction"] = tainted_body()["interaction"].clone();
        event["public_text"] = "MP-11 unmodified terminal/history text".into();
        let input = serde_json::json!({"type":"event","event_id":42,"event":event});
        if let Ok(output) = project_payload(input) {
            assert!(
                !output.to_string().contains(CANARY),
                "MP-11 secret escaped event {name}"
            );
            delivered += 1;
            assert_eq!(
                output["event"]["public_text"],
                "MP-11 unmodified terminal/history text"
            );
        }
    }
    assert_eq!(
        delivered, 20,
        "MP-11 current public events remain deliverable"
    );
}
#[test]
fn outbound_credential_mutations_and_unknown_payloads_fail_closed() {
    let safe = crate::runtime::kernel_access::error("sudo request refused").to_string();
    let frame = serde_json::json!({"type":"response","request_id":"sudo","response":null,
        "error":{"code":CANARY,"message":safe,"retryable":false}});
    let projected = project_payload(frame.clone()).unwrap();
    assert_eq!(projected["error"]["message"], safe);
    assert!(!projected.to_string().contains(CANARY));
    let mut tainted = frame;
    tainted["error"]["message"] = format!("{safe}: {CANARY}").into();
    assert!(!project_payload(tainted)
        .unwrap()
        .to_string()
        .contains(CANARY));
    for name in [
        "CredentialRegistered",
        "CredentialUpserted",
        "CredentialRemoved",
        "Credential",
        "CredentialsListed",
        "McpServer",
        "McpServersListed",
        "McpServerInstalled",
        "ConnectorRegistered",
        "ProviderLoginStarted",
        "FutureSecretResponse",
    ] {
        let input = serde_json::json!({"type":"response","request_id":"x","error":null,"response":{name:tainted_body()}});
        if let Ok(output) = project_payload(input) {
            assert!(!output.to_string().contains(CANARY), "MP-11 {name}");
        }
    }
    for input in [
        serde_json::json!({"type":"event","event_id":1,"event":{"event":"future_secret_event","value":CANARY}}),
        serde_json::json!({"type":"response","request_id":"x","error":null,"response":{"ok":true,"value":CANARY}}),
        serde_json::json!({"type":"response","request_id":"x","error":{"code":"test","message":CANARY,"retryable":false},"response":null}),
    ] {
        if let Ok(output) = project_payload(input) {
            assert!(!output.to_string().contains(CANARY));
        }
    }
}

#[test]
fn outbound_preserves_terminal_bytes_and_closed_transport_acknowledgments() {
    let unit = serde_json::json!({"type":"response","request_id":"logout",
        "response":"CloudRelayLoggedOut","error":null});
    assert_eq!(project_payload(unit.clone()).unwrap(), unit);
    assert!(
        project_payload(serde_json::json!({"type":"response","request_id":"future",
        "response":"FutureSecretResponse","error":null}))
        .is_err()
    );
    let input = serde_json::json!({"type":"event","event_id":7,"event":{"event":"terminal_output",
        "records":[{"bytes":[27,91,51,49,109,240,159,146,187],"text":"user transcript"}]}});
    assert_eq!(project_payload(input.clone()).unwrap(), input);
    for body in [
        serde_json::json!({"ok":true}),
        serde_json::json!({"ok":true,
        "resumed_from_event_id":7, "replay_gap":{"requested_from_event_id":7,
            "first_retained_event_id":10,"latest_event_id":12}}),
    ] {
        let input =
            serde_json::json!({"type":"response","request_id":"sub","response":body,"error":null});
        assert_eq!(project_payload(input.clone()).unwrap(), input);
    }
    assert!(
        project_payload(serde_json::json!({"type":"event","event_id":7,
        "event":{"event":"passkey_prompts_changed","prompts":[]}}))
        .is_err()
    );
}

#[test]
fn outbound_is_the_only_kernel_socket_writer() {
    let source = include_str!("../../runtime_transport.rs");
    let writer = source
        .split_once("async fn serve_kernel_socket")
        .unwrap()
        .1
        .split_once("fn incoming_frame_decode_error")
        .unwrap()
        .0;
    assert!(!writer.contains("writer.send("));
    assert!(!writer.contains("send_kernel_frame("));
    assert_eq!(
        include_str!("../outbound.rs")
            .matches("writer.send(")
            .count(),
        1
    );
}

#[test]
fn outbound_projected_browser_artifacts_keep_the_shared_chunk_budget() {
    // MP-08 / MP-10 / MP-11: projection must retain normal client semantics.
    let frame = serde_json::json!({"type":"response", "request_id":"large-artifact", "error":null,
    "response":{"RoomBrowserArtifact":{"result":{"payload":{
        "image_base64":"x".repeat(1024 * 1024), "width":1280, "height":720
    }}}}});
    let projected = project_payload(frame).unwrap();
    let encoded = crate::transport::kernel_protocol::serialize_frame_value(projected).unwrap();
    let decoded: Value = serde_json::from_str(&encoded).unwrap();
    assert!(decoded
        .pointer("/response/RoomBrowserArtifact/result/payload/image_base64")
        .is_none());
    assert_eq!(
        decoded["response"]["RoomBrowserArtifact"]["result"]["payload"]["width"],
        1280
    );
}

#[test]
fn outbound_preserves_kernel_error_codes_and_retries_without_error_payloads() {
    // MP-11: every mapped kernel code, plus explicit admission/cache errors.
    let source = include_str!("../../transport/kernel_protocol.rs");
    let mapped = source
        .split("kernel_error(\"")
        .skip(1)
        .map(|part| part.split('"').next().unwrap());
    for code in mapped.chain([
        "kernel_access_denied",
        "invalid_request",
        "invalid_frame",
        "kernel_request_overloaded",
        "duplicate_command_unavailable",
        "duplicate_command_conflict",
    ]) {
        for retryable in [false, true] {
            let frame = serde_json::json!({"type":"response","request_id":"error","response":null,
                "error":{"code":code,"message":CANARY,"retryable":retryable,"extra":CANARY}});
            let projected = project_payload(frame).unwrap();
            assert_eq!(projected["error"]["code"], code, "MP-11 {code}");
            assert_eq!(projected["error"]["retryable"], retryable);
            assert!(!projected.to_string().contains(CANARY));
        }
    }
    let message = crate::runtime::kernel_access::error("sudo request refused").to_string();
    let frame = serde_json::json!({"type":"response","request_id":"sudo","response":null,
        "error":{"code":"local_transport_error","message":message,"retryable":true}});
    assert_eq!(project_payload(frame.clone()).unwrap(), frame);
}
