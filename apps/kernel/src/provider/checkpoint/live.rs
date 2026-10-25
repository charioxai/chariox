//! Adapter driver for the real two-kernel/TUI drill. Inputs and payloads stay
//! in the caller's private evidence directory; provider accounts use the normal
//! product transfer path before this driver runs.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::{json, Value};

use super::ProviderNativeCheckpoint;
use crate::provider::{LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    operation: String,
    provider_run: Value,
    codex_home: PathBuf,
    artifact: PathBuf,
    output: PathBuf,
}

fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value
        .get(key)
        .and_then(Value::as_str)
        .expect("real provider-run field")
}

fn save(path: &std::path::Path, value: &impl serde::Serialize) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .expect("create private drill output");
    serde_json::to_writer(&mut file, value).expect("write private drill output");
    file.write_all(b"\n").unwrap();
    file.sync_all().unwrap();
}

struct UtilityCleanup;
impl Drop for UtilityCleanup {
    fn drop(&mut self) {
        crate::provider::shutdown_codex_account_endpoints();
    }
}

#[test]
#[ignore = "requires two real disposable kernels and an explicit private drill manifest"]
fn live_codex_native_checkpoint_driver() {
    let _utilities = UtilityCleanup;
    let input_path =
        std::env::var_os("CHARIOX_NATIVE_CHECKPOINT_DRILL").expect("explicit drill manifest");
    let input: Input =
        serde_json::from_reader(File::open(input_path).unwrap()).expect("private drill manifest");
    assert!(
        input.codex_home.is_absolute()
            && input.artifact.is_absolute()
            && input.output.is_absolute()
    );
    let public = &input.provider_run;
    let mut request = LaunchProviderRequest::new(
        field(public, "session_id"),
        field(public, "adapter_key"),
        field(public, "provider"),
        field(public, "account_profile"),
        field(public, "model"),
    );
    request = request.with_owner_user_id(field(public, "owner_user_id"));
    request = request.with_execution_mode(
        serde_json::from_value(public["execution_mode"].clone()).expect("real execution mode"),
    );
    request = request.with_permission_level(
        serde_json::from_value(public["permission_level"].clone()).expect("real permission level"),
    );
    request = request.with_client_interface(
        serde_json::from_value(public["client_interface"].clone()).expect("real client interface"),
    );
    request = request.with_variant(
        public
            .get("variant")
            .and_then(Value::as_str)
            .map(str::to_string),
    );
    if let Some(id) = public.get("agent_instance_id").and_then(Value::as_str) {
        request = request.with_agent_id(id);
    }
    let directory = PathBuf::from(field(public, "working_directory"));
    request = request.with_working_directory(directory.clone());
    let launch = ProviderLaunchResult {
        endpoint_mode: serde_json::from_value(public["endpoint_mode"].clone())
            .expect("real endpoint mode"),
        process_label: "native-checkpoint-drill".to_string(),
        pty_target: None,
        pty_program: None,
        pty_args: Vec::new(),
        pty_env: BTreeMap::from([(
            "CODEX_HOME".to_string(),
            input.codex_home.display().to_string(),
        )]),
        pty_env_remove: Vec::new(),
        working_directory: Some(directory),
        structured_endpoint: Some(field(public, "structured_endpoint").to_string()),
    };
    match input.operation.as_str() {
        "export" => {
            assert_eq!(
                field(public, "state"),
                "Ended",
                "source must have a real teardown receipt"
            );
            let mut run = RuntimeProviderRun::new(field(public, "id"), &request, launch);
            run.set_provider_session_id(Some(field(public, "provider_session_id").to_string()));
            run.mark_ended();
            let checkpoint =
                ProviderNativeCheckpoint::export(&run).expect("export real stopped native session");
            save(&input.artifact, &checkpoint);
            save(
                &input.output,
                &json!({"binding": checkpoint.binding(), "exported": true, "payload_printed": false}),
            );
        }
        "import" => {
            let file = File::open(&input.artifact).expect("private checkpoint artifact");
            assert!(
                file.metadata().unwrap().len()
                    <= (super::MAX_NATIVE_CHECKPOINT_BYTES as u64 * 4 + 1024 * 1024)
            );
            let checkpoint: ProviderNativeCheckpoint =
                serde_json::from_reader(file).expect("decode private checkpoint artifact");
            let state = checkpoint
                .import(&request, &launch)
                .expect("stage and resume real native checkpoint");
            assert_eq!(
                state.codex_thread_id(),
                Some(checkpoint.provider_session_id())
            );
            assert!(state.required_native_session().is_some());
            let after = super::files::Directory::open(&input.codex_home)
                .unwrap()
                .read(&checkpoint.relative_path)
                .unwrap();
            let unchanged_prefix = after.starts_with(&checkpoint.contents);
            let appended_record_types: Vec<_> = if unchanged_prefix {
                after[checkpoint.contents.len()..]
                    .split(|byte| *byte == b'\n')
                    .filter(|line| !line.is_empty())
                    .map(|line| {
                        serde_json::from_slice::<Value>(line)
                            .ok()
                            .and_then(|record| record["type"].as_str().map(str::to_string))
                            .unwrap_or_else(|| "invalid-json".to_string())
                    })
                    .collect()
            } else {
                Vec::new()
            };
            save(
                &input.output,
                &json!({"imported":true,"same_native_session":true,"required_resume":true,"payload_printed":false,
                    "artifact_before_bytes":checkpoint.contents.len(),"artifact_after_bytes":after.len(),
                    "artifact_before_sha256":checkpoint.sha256(),"artifact_after_sha256":super::digest(&after),
                    "unchanged_prefix":unchanged_prefix,"appended_record_types":appended_record_types}),
            );
        }
        "delete" => {
            let endpoint = crate::provider::ensure_codex_account_endpoint(
                &request.owner_user_id,
                &request.account_profile,
                BTreeMap::from([(
                    "CODEX_HOME".to_string(),
                    input.codex_home.display().to_string(),
                )]),
            )
            .expect("open owned native cleanup utility");
            let client =
                crate::provider::CodexClient::new("native-checkpoint-cleanup", endpoint).unwrap();
            let mut socket = client
                .connect_initialized()
                .expect("connect cleanup utility");
            let _: Value = client
                .send_request(
                    &mut socket,
                    &mut 1,
                    "thread/delete",
                    json!({"threadId": field(public, "provider_session_id")}),
                )
                .expect("delete only the drill's native session");
            save(
                &input.output,
                &json!({"native_session_removed":true,"payload_printed":false}),
            );
        }
        _ => panic!("unsupported drill operation"),
    }
}
