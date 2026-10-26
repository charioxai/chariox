//! MP-08 / MP-09 / MP-10 / MP-11: test-only admitted peer for the G09/G10 hosted drills.
//!
//! The drill driver (a Node script) starts real kernels against a real relay, then runs this
//! test binary with `--ignored`. Each sender is the disposable state directory of a real
//! kernel, so the relay and the target kernel see a genuine admitted peer. The driver writes
//! one JSON command per stdin line and reads one `RECEIPT` line per command.
//! Nothing here is reachable from product code.
use super::*;
use crate::config::DaemonConfig;
use crate::runtime::cloud_api_client::{
    issue_cloud_runtime_token, CloudRuntimeTokenRequestOptions,
};
use crate::runtime::cloud_relay_control::cloud_runtime_token_subject;
use crate::transport::relay_peer::RelayPeerRequest;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::BufRead;

const SENDERS_ENV: &str = "CHARIOX_ADMISSION_DRILL_SENDERS";

#[derive(Deserialize)]
struct SenderSpec {
    name: String,
    env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Command {
    id: String,
    sender: String,
    target_kernel_id: String,
    request: serde_json::Value,
}

fn load_sender(spec: &SenderSpec) -> DaemonConfig {
    for (key, value) in &spec.env {
        std::env::set_var(key, value);
    }
    DaemonConfig::load_from_env()
}

async fn with_fresh_token(config: &DaemonConfig) -> Result<DaemonConfig, String> {
    let profile = config
        .cloud_relay
        .clone()
        .ok_or("sender kernel has no Cloud relay profile; enroll it first")?;
    let subject = cloud_runtime_token_subject(config, &profile);
    let issued = issue_cloud_runtime_token(
        &profile,
        &subject.subject,
        subject.subject_kind,
        CloudRuntimeTokenRequestOptions {
            machine_id: subject.machine_id,
            public_key_thumbprint: Some(crate::runtime::terminal_pairings::public_key_thumbprint(
                &config.relay_public_key,
            )),
            ..CloudRuntimeTokenRequestOptions::default()
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    let mut config = config.clone();
    config.relay_url = Some(profile.relay_url);
    config.relay_token = Some(issued.token);
    Ok(config)
}

fn receipt(id: &str, outcome: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "id": id, "at_ms": crate::session::unix_epoch_ms(), "outcome": outcome })
}

async fn send(config: &DaemonConfig, command: &Command) -> serde_json::Value {
    let request: RelayPeerRequest = match serde_json::from_value(command.request.clone()) {
        Ok(request) => request,
        Err(error) => {
            return serde_json::json!({ "kind": "bad_command", "message": error.to_string() })
        }
    };
    let config = match with_fresh_token(config).await {
        Ok(config) => config,
        Err(message) => return serde_json::json!({ "kind": "token_failed", "message": message }),
    };
    let target = ClientTarget {
        daemon_id: Some(command.target_kernel_id.clone()),
        daemon_alias: None,
    };
    match send_peer_request_via_temporary_connection(&config, target, request).await {
        Ok(response) => serde_json::json!({
            "kind": "accepted",
            "response": serde_json::to_value(&response).unwrap_or_default(),
        }),
        Err(DaemonError::RelayTransport {
            code,
            message,
            retryable,
            ..
        }) => serde_json::json!({
            "kind": "refused", "code": code, "message": message, "retryable": retryable,
        }),
        Err(error) => serde_json::json!({ "kind": "local_error", "message": error.to_string() }),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "hosted drill: needs real kernels and a relay; run by the admission drill driver"]
async fn admission_drill_peer() {
    let specs: Vec<SenderSpec> = serde_json::from_str(
        &std::env::var(SENDERS_ENV)
            .expect("CHARIOX_ADMISSION_DRILL_SENDERS must hold the sender list"),
    )
    .expect("sender list must be JSON");
    let senders: BTreeMap<String, DaemonConfig> = specs
        .iter()
        .map(|spec| (spec.name.clone(), load_sender(spec)))
        .collect();
    println!(
        "READY {}",
        serde_json::json!(senders.keys().collect::<Vec<_>>())
    );
    let (lines_tx, mut lines_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if lines_tx.send(line).is_err() {
                break;
            }
        }
    });
    while let Some(line) = lines_rx.recv().await {
        if line.trim().is_empty() {
            continue;
        }
        let outcome = match serde_json::from_str::<Command>(&line) {
            Ok(command) => match senders.get(&command.sender) {
                Some(config) => receipt(&command.id, send(config, &command).await),
                None => receipt(
                    &command.id,
                    serde_json::json!({ "kind": "bad_command", "message": "unknown sender" }),
                ),
            },
            Err(error) => receipt(
                "?",
                serde_json::json!({ "kind": "bad_command", "message": error.to_string() }),
            ),
        };
        println!("RECEIPT {outcome}");
    }
}
