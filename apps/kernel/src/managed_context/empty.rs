use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{watch, RwLock};

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::managed_bootstrap::ConfirmedManagedKernelRegistration;
use crate::managed_context::cloud_completion::{
    complete_managed_context_import, context_manifest_digest,
    validate_managed_context_completion_binding,
};
use crate::managed_context::kernel::configured_managed_kernel_context_paths;
use crate::managed_context::package::ManagedContextPlanBinding;
use crate::transport::relay_client::RelayClientState;

const EMPTY_CONTEXT_RECEIPT_SCHEMA_VERSION: u32 = 1;
const EMPTY_CONTEXT_RECEIPT_MAX_BYTES: u64 = 8 * 1024;
const MIN_RETRY_DELAY: Duration = Duration::from_secs(1);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);
const RELAY_STATE_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub(crate) struct EmptyManagedContextCompletion {
    config: DaemonConfig,
    registration: ConfirmedManagedKernelRegistration,
    plan: ManagedContextPlanBinding,
    manifest_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EmptyManagedContextReceipt {
    schema_version: u32,
    kind: EmptyManagedContextKind,
    environment_id: String,
    machine_id: String,
    kernel_id: String,
    context_id: String,
    plan_digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EmptyManagedContextKind {
    Empty,
}

impl EmptyManagedContextCompletion {
    pub(crate) fn prepare(
        config: &DaemonConfig,
        registration: Option<&ConfirmedManagedKernelRegistration>,
    ) -> Result<Option<Self>, DaemonError> {
        let Some(registration) = registration else {
            return Ok(None);
        };
        let Some(context_plan) = registration.context_plan.as_ref() else {
            return Ok(None);
        };
        if !context_plan.is_empty() {
            return Ok(None);
        }
        let plan = context_plan.package_binding();
        validate_managed_context_completion_binding(config, registration, &plan)?;
        let receipt = expected_receipt(registration, &plan);
        let receipt_path = empty_context_receipt_path(config)?;
        let (capability_root, vault_path) = configured_managed_kernel_context_paths()?;
        let receipt_exists = read_receipt(&receipt_path)?.is_some();
        let workspace = ensure_empty_managed_context_workspace(config, &plan.context_id)?;
        if !receipt_exists {
            validate_empty_workspace(&workspace)?;
        }
        let receipt_json =
            load_or_create_empty_receipt(&receipt_path, &receipt, &capability_root, &vault_path)?;
        let manifest_digest = context_manifest_digest(&receipt_json)?;
        Ok(Some(Self {
            config: config.clone(),
            registration: registration.clone(),
            plan,
            manifest_digest,
        }))
    }

    pub(crate) async fn run(
        self,
        relay_state: Arc<RwLock<RelayClientState>>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let mut retry_delay = MIN_RETRY_DELAY;
        loop {
            if wait_for_relay_connection(&relay_state, &mut shutdown).await {
                return;
            }
            match complete_managed_context_import(
                &self.config,
                &self.registration,
                &self.plan,
                &self.manifest_digest,
            )
            .await
            {
                Ok(()) => {
                    crate::logging::info_with_fields(
                        "managed_context.empty_ready",
                        "empty managed kernel context is ready",
                        serde_json::json!({
                            "environment_id": self.registration.environment_id,
                            "kernel_id": self.registration.kernel_id,
                            "context_id": self.plan.context_id,
                            "context_manifest_digest": self.manifest_digest,
                        }),
                    );
                    return;
                }
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "managed_context.empty_completion_failed",
                        "empty managed kernel context completion failed; kernel will retry",
                        serde_json::json!({
                            "environment_id": self.registration.environment_id,
                            "kernel_id": self.registration.kernel_id,
                            "context_id": self.plan.context_id,
                            "error": error.to_string(),
                            "retry_delay_ms": retry_delay.as_millis(),
                        }),
                    );
                }
            }
            if wait_or_shutdown(&mut shutdown, jittered(retry_delay)).await {
                return;
            }
            retry_delay = retry_delay.saturating_mul(2).min(MAX_RETRY_DELAY);
        }
    }
}

pub(crate) fn empty_managed_context_workspace_path(
    config: &DaemonConfig,
    context_id: &str,
) -> Result<PathBuf, DaemonError> {
    let state_root = config
        .durable_state_path()
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| empty_context_error("durable state path has no parent directory"))?;
    Ok(state_root
        .join("managed-context-empty-workspaces")
        .join(format!("{:x}", Sha256::digest(context_id.as_bytes())))
        .join("workspace"))
}

pub(crate) fn ensure_empty_managed_context_workspace(
    config: &DaemonConfig,
    context_id: &str,
) -> Result<PathBuf, DaemonError> {
    let workspace = empty_managed_context_workspace_path(config, context_id)?;
    let durable_state_path = config.durable_state_path();
    let state_root = durable_state_path
        .parent()
        .ok_or_else(|| empty_context_error("durable state path has no parent directory"))?;
    let workspace_root = workspace
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| empty_context_error("empty workspace path has no managed root"))?;
    ensure_real_private_directory(state_root)?;
    ensure_real_private_directory(workspace_root)?;
    ensure_real_private_directory(
        workspace
            .parent()
            .ok_or_else(|| empty_context_error("empty workspace path has no context root"))?,
    )?;
    ensure_real_private_directory(&workspace)?;
    let canonical_state_root = fs::canonicalize(state_root)
        .map_err(|error| empty_context_io_error("resolve managed state root", error))?;
    let canonical_workspace = fs::canonicalize(&workspace)
        .map_err(|error| empty_context_io_error("resolve empty managed workspace", error))?;
    if !canonical_workspace.starts_with(&canonical_state_root) {
        return Err(empty_context_error(
            "empty managed workspace escapes the managed state root",
        ));
    }
    Ok(canonical_workspace)
}

fn validate_empty_workspace(workspace: &Path) -> Result<(), DaemonError> {
    let mut entries = fs::read_dir(workspace)
        .map_err(|error| empty_context_io_error("inspect empty managed workspace", error))?;
    if entries
        .next()
        .transpose()
        .map_err(|error| empty_context_io_error("inspect empty managed workspace", error))?
        .is_some()
    {
        return Err(empty_context_error(
            "empty managed workspace is not pristine before initialization",
        ));
    }
    Ok(())
}

fn ensure_real_private_directory(path: &Path) -> Result<(), DaemonError> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(empty_context_io_error(
                "create empty managed workspace directory",
                error,
            ))
        }
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        empty_context_io_error("inspect empty managed workspace directory", error)
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(empty_context_error(
            "empty managed workspace path must be a real directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            empty_context_io_error("secure empty managed workspace directory", error)
        })?;
    }
    Ok(())
}

fn expected_receipt(
    registration: &ConfirmedManagedKernelRegistration,
    plan: &ManagedContextPlanBinding,
) -> EmptyManagedContextReceipt {
    EmptyManagedContextReceipt {
        schema_version: EMPTY_CONTEXT_RECEIPT_SCHEMA_VERSION,
        kind: EmptyManagedContextKind::Empty,
        environment_id: registration.environment_id.clone(),
        machine_id: registration.machine_id.clone(),
        kernel_id: registration.kernel_id.clone(),
        context_id: plan.context_id.clone(),
        plan_digest: plan.plan_digest.clone(),
    }
}

fn empty_context_receipt_path(config: &DaemonConfig) -> Result<PathBuf, DaemonError> {
    config
        .durable_state_path()
        .parent()
        .map(|root| root.join("managed-empty-context").join("receipt.json"))
        .ok_or_else(|| empty_context_error("durable state path has no parent directory"))
}

fn load_or_create_empty_receipt(
    path: &Path,
    expected: &EmptyManagedContextReceipt,
    capability_root: &Path,
    vault_path: &Path,
) -> Result<String, DaemonError> {
    match read_receipt(path)? {
        Some((receipt, json)) => {
            if &receipt != expected {
                return Err(empty_context_error(
                    "persisted empty context receipt does not match this managed kernel",
                ));
            }
            Ok(json)
        }
        None => {
            validate_pristine_context(capability_root, vault_path)?;
            let bytes = serde_json::to_vec(expected).map_err(|error| {
                empty_context_error(format!("serialize empty context receipt: {error}"))
            })?;
            crate::config::write_private_file(path, &bytes)
                .map_err(|error| empty_context_io_error("persist empty context receipt", error))?;
            let (persisted, json) = read_receipt(path)?.ok_or_else(|| {
                empty_context_error("empty context receipt disappeared after persistence")
            })?;
            if &persisted != expected {
                return Err(empty_context_error(
                    "persisted empty context receipt changed during creation",
                ));
            }
            Ok(json)
        }
    }
}

fn validate_pristine_context(capability_root: &Path, vault_path: &Path) -> Result<(), DaemonError> {
    match fs::symlink_metadata(capability_root) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(empty_context_error(
                    "managed capability isolation root is not a real directory",
                ));
            }
            let mut entries = fs::read_dir(capability_root).map_err(|error| {
                empty_context_io_error("inspect managed capability isolation root", error)
            })?;
            if entries
                .next()
                .transpose()
                .map_err(|error| {
                    empty_context_io_error("inspect managed capability isolation root", error)
                })?
                .is_some()
            {
                return Err(empty_context_error(
                    "managed capability isolation root is not empty",
                ));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(empty_context_io_error(
                "inspect managed capability isolation root",
                error,
            ))
        }
    }
    match fs::symlink_metadata(vault_path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(empty_context_error(
            "managed Vault is not empty before Empty context initialization",
        )),
        Err(error) => Err(empty_context_io_error("inspect managed Vault", error)),
    }
}

fn read_receipt(path: &Path) -> Result<Option<(EmptyManagedContextReceipt, String)>, DaemonError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(empty_context_io_error("open empty context receipt", error)),
    };
    validate_receipt_file(&file)?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(EMPTY_CONTEXT_RECEIPT_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| empty_context_io_error("read empty context receipt", error))?;
    if bytes.is_empty() || bytes.len() as u64 > EMPTY_CONTEXT_RECEIPT_MAX_BYTES {
        return Err(empty_context_error(
            "empty context receipt exceeds its size limit",
        ));
    }
    validate_receipt_file(&file)?;
    let receipt = serde_json::from_slice::<EmptyManagedContextReceipt>(&bytes)
        .map_err(|_| empty_context_error("empty context receipt is invalid"))?;
    let json = String::from_utf8(bytes)
        .map_err(|_| empty_context_error("empty context receipt is not UTF-8"))?;
    Ok(Some((receipt, json)))
}

fn validate_receipt_file(file: &File) -> Result<(), DaemonError> {
    let metadata = file
        .metadata()
        .map_err(|error| empty_context_io_error("inspect empty context receipt", error))?;
    if !metadata.is_file() || metadata.len() > EMPTY_CONTEXT_RECEIPT_MAX_BYTES {
        return Err(empty_context_error(
            "empty context receipt is not a bounded regular file",
        ));
    }
    Ok(())
}

async fn wait_for_relay_connection(
    relay_state: &Arc<RwLock<RelayClientState>>,
    shutdown: &mut watch::Receiver<bool>,
) -> bool {
    loop {
        if *shutdown.borrow() {
            return true;
        }
        if relay_state.read().await.connected() {
            return false;
        }
        if wait_or_shutdown(shutdown, RELAY_STATE_POLL_INTERVAL).await {
            return true;
        }
    }
}

async fn wait_or_shutdown(shutdown: &mut watch::Receiver<bool>, delay: Duration) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(delay) => false,
        changed = shutdown.changed() => changed.is_ok() && *shutdown.borrow(),
    }
}

fn jittered(delay: Duration) -> Duration {
    let factor = rand::thread_rng().gen_range(0.8_f64..=1.2_f64);
    Duration::from_secs_f64(delay.as_secs_f64() * factor)
}

fn empty_context_error(message: impl Into<String>) -> DaemonError {
    DaemonError::ManagedContext {
        code: "managed_context_empty_invalid",
        operation: "initialize empty managed context",
        message: message.into(),
        retryable: false,
    }
}

fn empty_context_io_error(operation: &'static str, error: io::Error) -> DaemonError {
    DaemonError::ManagedContext {
        code: "managed_context_empty_unavailable",
        operation,
        message: error.to_string(),
        retryable: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PersistedCloudRelayProfile;
    use crate::managed_bootstrap::ManagedKernelContextPlan;
    use crate::transport::relay_client::RelayOutgoingSender;
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn empty_receipt_is_stable_after_user_context_changes() {
        let root = test_root("stable");
        let capability_root = root.join("capabilities");
        let vault_path = root.join("vault.json");
        let config = test_config(&root, "machine-empty", "kernel-empty", "http://127.0.0.1:9");
        let registration = test_registration("machine-empty", "kernel-empty", "context-empty");
        let plan = registration
            .context_plan
            .as_ref()
            .expect("context plan")
            .package_binding();
        let receipt_path = empty_context_receipt_path(&config).expect("receipt path");
        let expected = expected_receipt(&registration, &plan);

        let first =
            load_or_create_empty_receipt(&receipt_path, &expected, &capability_root, &vault_path)
                .expect("create empty receipt");
        fs::create_dir_all(&capability_root).expect("create capability root");
        fs::write(capability_root.join("user-extension.json"), b"later")
            .expect("write later Extension");
        fs::write(&vault_path, b"later encrypted Vault").expect("write later Vault");
        let second =
            load_or_create_empty_receipt(&receipt_path, &expected, &capability_root, &vault_path)
                .expect("reuse empty receipt after later changes");

        assert_eq!(first, second);
        assert_eq!(
            context_manifest_digest(&first).expect("manifest digest"),
            context_manifest_digest(&second).expect("manifest digest")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn empty_receipt_rejects_dirty_initial_context_and_identity_rebinding() {
        let root = test_root("dirty");
        let capability_root = root.join("capabilities");
        let vault_path = root.join("vault.json");
        fs::create_dir_all(&capability_root).expect("create capability root");
        fs::write(capability_root.join("stale"), b"stale").expect("write stale capability");
        let config = test_config(&root, "machine-empty", "kernel-empty", "http://127.0.0.1:9");
        let registration = test_registration("machine-empty", "kernel-empty", "context-empty");
        let plan = registration
            .context_plan
            .as_ref()
            .unwrap()
            .package_binding();
        let receipt_path = empty_context_receipt_path(&config).expect("receipt path");
        let expected = expected_receipt(&registration, &plan);
        assert!(load_or_create_empty_receipt(
            &receipt_path,
            &expected,
            &capability_root,
            &vault_path,
        )
        .is_err());

        fs::remove_dir_all(&capability_root).expect("remove dirty context");
        fs::write(&vault_path, b"stale encrypted Vault").expect("write stale Vault");
        assert!(load_or_create_empty_receipt(
            &receipt_path,
            &expected,
            &capability_root,
            &vault_path,
        )
        .is_err());
        fs::remove_file(&vault_path).expect("remove stale Vault");
        load_or_create_empty_receipt(&receipt_path, &expected, &capability_root, &vault_path)
            .expect("create clean receipt");
        let rebound = test_registration("machine-empty", "kernel-rebound", "context-empty");
        let rebound_plan = rebound.context_plan.as_ref().unwrap().package_binding();
        assert!(load_or_create_empty_receipt(
            &receipt_path,
            &expected_receipt(&rebound, &rebound_plan),
            &capability_root,
            &vault_path,
        )
        .is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn empty_completion_waits_for_relay_and_reports_the_stable_digest() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind Cloud fixture");
        let api_url = format!("http://{}", listener.local_addr().expect("fixture address"));
        let registration = test_registration("machine-empty", "kernel-empty", "context-empty");
        let plan = registration
            .context_plan
            .as_ref()
            .unwrap()
            .package_binding();
        let receipt_json = serde_json::to_string(&expected_receipt(&registration, &plan))
            .expect("serialize expected receipt");
        let manifest_digest = context_manifest_digest(&receipt_json).expect("manifest digest");
        let completion = EmptyManagedContextCompletion {
            config: test_config(
                &test_root("relay"),
                "machine-empty",
                "kernel-empty",
                &api_url,
            ),
            registration,
            plan,
            manifest_digest: manifest_digest.clone(),
        };
        let accepted = Arc::new(AtomicBool::new(false));
        let accepted_fixture = accepted.clone();
        let expected_digest = manifest_digest.clone();
        let fixture = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept completion request");
            accepted_fixture.store(true, Ordering::SeqCst);
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set fixture timeout");
            let body = read_http_json_body(&mut stream);
            assert_eq!(body["accountId"], "account-1");
            assert_eq!(body["environmentId"], "environment-empty");
            assert_eq!(body["machineId"], "machine-empty");
            assert_eq!(body["kernelId"], "kernel-empty");
            assert_eq!(body["contextId"], "context-empty");
            assert_eq!(body["contextManifestDigest"], expected_digest);
            let response = serde_json::to_vec(&serde_json::json!({
                "ready": true,
                "observedState": "ready",
                "contextManifestDigest": body["contextManifestDigest"],
            }))
            .expect("serialize completion response");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                response.len()
            )
            .expect("write completion headers");
            stream
                .write_all(&response)
                .expect("write completion response");
        });
        let relay_state = Arc::new(RwLock::new(RelayClientState::default()));
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let completion_task = tokio::spawn(completion.run(relay_state.clone(), shutdown_rx));

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!accepted.load(Ordering::SeqCst));
        let (outgoing, _priority_rx, _event_rx) = RelayOutgoingSender::channel(1);
        relay_state
            .write()
            .await
            .test_set_connected_sender(outgoing, "wss://relay.example.test");

        tokio::time::timeout(Duration::from_secs(5), completion_task)
            .await
            .expect("empty completion timeout")
            .expect("empty completion task");
        fixture.join().expect("Cloud fixture");
        assert!(accepted.load(Ordering::SeqCst));
    }

    fn read_http_json_body(stream: &mut std::net::TcpStream) -> serde_json::Value {
        let mut request = Vec::new();
        let mut chunk = [0_u8; 2048];
        loop {
            let read = stream.read(&mut chunk).expect("read completion request");
            assert!(read > 0, "completion request ended before its body");
            request.extend_from_slice(&chunk[..read]);
            let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            assert!(headers.starts_with("POST /v1/managed-kernels/context/complete "));
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .expect("completion content length");
            if request.len() < header_end + 4 + content_length {
                continue;
            }
            return serde_json::from_slice(
                &request[header_end + 4..header_end + 4 + content_length],
            )
            .expect("decode completion body");
        }
    }

    fn test_registration(
        machine_id: &str,
        kernel_id: &str,
        context_id: &str,
    ) -> ConfirmedManagedKernelRegistration {
        ConfirmedManagedKernelRegistration {
            environment_id: "environment-empty".to_string(),
            machine_id: machine_id.to_string(),
            kernel_id: kernel_id.to_string(),
            context_plan: Some(ManagedKernelContextPlan::empty_for_tests(context_id)),
        }
    }

    fn test_config(root: &Path, machine_id: &str, kernel_id: &str, api_url: &str) -> DaemonConfig {
        let mut config = DaemonConfig::for_tests();
        config.host_machine_id = machine_id.to_string();
        config.daemon_id = kernel_id.to_string();
        config.user_config.state.path = Some(root.join("state.db").display().to_string());
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            api_url: api_url.to_string(),
            email: "user@example.test".to_string(),
            account_id: "account-1".to_string(),
            user_id: "user-1".to_string(),
            account_slug: "account".to_string(),
            realm_id: "realm-1".to_string(),
            relay_url: "wss://relay.example.test".to_string(),
            issuer_id: "issuer-1".to_string(),
            client_id: None,
            client_alias: None,
            machine_id: Some(machine_id.to_string()),
            machine_alias: Some("Managed empty".to_string()),
            machine_credential: Some(format!("mcred_{}", "m".repeat(43))),
            cloud_session_token: None,
            cloud_session_expires_at_ms: None,
            token_expires_at_ms: None,
        });
        config
    }

    fn test_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "chariox-empty-context-{label}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ))
    }
}
