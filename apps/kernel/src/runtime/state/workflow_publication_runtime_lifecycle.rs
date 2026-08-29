//! Kernel-owned lifecycle control for local workflow publication runtimes.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use base64::Engine as _;
use rand::RngCore;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream as TokioTcpStream;
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock};
use tokio::time::{sleep, Duration, Instant};

use crate::error::DaemonError;
use crate::local::{
    BindWorkflowPublicationDeploymentRequest, ControlWorkflowPublicationRuntimeRequest,
    LocalDaemonResponse, RegisterWorkflowPublicationEndpointRequest,
    WorkflowPublicationRuntimeAction,
};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::session::WorkflowPublicationDefinition;
use crate::transport::relay_client::RelayClientState;

use super::KernelRuntimeState;

const DEFAULT_PUBLICATION_RUNTIME_HOST: &str = "127.0.0.1";
const DEFAULT_PUBLICATION_RUNTIME_PORT: u16 = 3000;
const PUBLICATION_RUNTIME_RECOVERY_BASE_DELAY_MS: u64 = 1_000;
const PUBLICATION_RUNTIME_RECOVERY_MAX_DELAY_MS: u64 = 60_000;
// A source checkout may need to build the TypeScript gateway before it can
// listen. Keep the readiness deadline long enough for that one-time build;
// subsequent launches remain effectively immediate.
const PUBLICATION_RUNTIME_START_TIMEOUT: Duration = Duration::from_secs(60);
const PUBLICATION_RUNTIME_START_POLL: Duration = Duration::from_millis(50);

#[derive(Clone, Default)]
pub(crate) struct WorkflowPublicationRuntimeProcessStore {
    inner: Arc<Mutex<BTreeMap<String, WorkflowPublicationRuntimeProcess>>>,
    launching: Arc<Mutex<BTreeSet<String>>>,
    recoveries: Arc<Mutex<BTreeMap<String, WorkflowPublicationRuntimeRecovery>>>,
}

struct WorkflowPublicationRuntimeProcess {
    child: Child,
    process_id: Option<u32>,
    host: String,
    port: u16,
    local_url: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct WorkflowPublicationRuntimeRecovery {
    failures: u32,
    next_attempt_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkflowPublicationDeploymentBinding {
    setup_id: String,
    operation_key: String,
    deployment_id: String,
    environment_id: String,
    release_id: String,
    package_digest: String,
    desired_revision: u64,
    caller_claims_public_key_pem: String,
}

#[derive(Default)]
struct PublicationRuntimeLaunchContext {
    cloud_deployment_id: Option<String>,
    expected_package_digest: Option<String>,
    binding: Option<WorkflowPublicationDeploymentBinding>,
}

pub(crate) async fn execute_control_workflow_publication_runtime_request(
    runtime_state: &KernelRuntimeState,
    request: ControlWorkflowPublicationRuntimeRequest,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    let publication = runtime_state
        .owned
        .session_store
        .read()
        .resolve_workflow_publication_ref(&request.session_id, &request.publication_ref)?;
    if publication.created_by_user_id() != caller_user_id {
        return Err(super::KernelRuntimeOwnedState::deny_owner(
            caller_user_id,
            publication.created_by_user_id(),
            format!("workflow publication `{}`", request.publication_ref),
            "control workflow publication runtime",
        ));
    }
    let publication_id = publication.id().to_string();
    let process_key = publication_runtime_process_key(&request.session_id, &publication_id);
    match request.action {
        WorkflowPublicationRuntimeAction::Inspect => {
            inspect_publication_runtime(runtime_state, publication, process_key).await
        }
        WorkflowPublicationRuntimeAction::Stop => {
            stop_publication_runtime(runtime_state, &process_key).await?;
            let publication = mark_publication_runtime_status(
                runtime_state,
                &request.session_id,
                &publication_id,
                "stopped",
                Some(None),
                Some(stopped_publication_runtime_metadata(&publication, true)),
            )?;
            Ok(LocalDaemonResponse::WorkflowPublicationRuntimeControlled {
                publication,
                action: WorkflowPublicationRuntimeAction::Stop,
                status: "stopped".to_string(),
                local_url: None,
                open_url: None,
                viewer_url: None,
                process_id: None,
                message: Some("publication runtime stopped".to_string()),
            })
        }
        WorkflowPublicationRuntimeAction::Start | WorkflowPublicationRuntimeAction::Restart => {
            if request.action == WorkflowPublicationRuntimeAction::Restart {
                stop_publication_runtime(runtime_state, &process_key).await?;
            }
            start_publication_runtime(
                runtime_state,
                request,
                publication,
                process_key,
                PublicationRuntimeLaunchContext::default(),
            )
            .await
        }
    }
}

pub(crate) async fn execute_bind_workflow_publication_deployment_request(
    runtime_state: &KernelRuntimeState,
    config_projection: &DaemonConfigProjectionStore,
    relay_state: Arc<RwLock<RelayClientState>>,
    request: BindWorkflowPublicationDeploymentRequest,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    let binding = validated_deployment_binding(&request)?;
    let publication = runtime_state
        .owned
        .session_store
        .read()
        .resolve_workflow_publication_ref(&request.session_id, &request.publication_ref)?;
    if publication.created_by_user_id() != caller_user_id {
        return Err(super::KernelRuntimeOwnedState::deny_owner(
            caller_user_id,
            publication.created_by_user_id(),
            format!("workflow publication `{}`", request.publication_ref),
            "bind workflow publication deployment",
        ));
    }
    if !publication.enabled() {
        return Err(publication_runtime_error(
            "bind workflow publication deployment",
            "disabled workflow publications cannot be deployed",
        ));
    }
    let publication_id = publication.id().to_string();
    let process_key = publication_runtime_process_key(&request.session_id, &publication_id);
    let existing_binding = publication_deployment_binding(&publication);
    if let Some(existing) = existing_binding.as_ref() {
        if existing.operation_key == binding.operation_key {
            if existing != &binding {
                return Err(publication_runtime_error(
                    "bind workflow publication deployment",
                    "deployment bind operation key is already associated with different deployment facts",
                ));
            }
            if let Some(running) = runtime_state
                .owned
                .workflow_publication_runtimes
                .running(&process_key)
                .await?
            {
                return Ok(bound_publication_response(
                    publication,
                    binding,
                    running.local_url,
                    running.process_id,
                    true,
                ));
            }
        }
    }

    if runtime_state
        .owned
        .workflow_publication_runtimes
        .running(&process_key)
        .await?
        .is_some()
    {
        stop_publication_runtime(runtime_state, &process_key).await?;
    }
    let port = if is_schedule_only_publication(&publication) {
        None
    } else {
        Some(reserve_ephemeral_publication_runtime_port()?)
    };
    let launch = start_publication_runtime(
        runtime_state,
        ControlWorkflowPublicationRuntimeRequest {
            session_id: request.session_id.clone(),
            publication_ref: publication_id.clone(),
            action: WorkflowPublicationRuntimeAction::Start,
            host: Some(DEFAULT_PUBLICATION_RUNTIME_HOST.to_string()),
            port,
            kernel_url: None,
        },
        publication,
        process_key,
        PublicationRuntimeLaunchContext {
            cloud_deployment_id: Some(binding.deployment_id.clone()),
            expected_package_digest: Some(binding.package_digest.clone()),
            binding: Some(binding.clone()),
        },
    )
    .await?;
    let LocalDaemonResponse::WorkflowPublicationRuntimeControlled {
        publication,
        local_url,
        process_id,
        ..
    } = launch
    else {
        return Err(publication_runtime_error(
            "bind workflow publication deployment",
            "publication runtime launch returned an unexpected response",
        ));
    };
    let Some(local_url) = local_url else {
        return Ok(bound_publication_response(
            publication,
            binding,
            None,
            process_id,
            false,
        ));
    };
    let registered = super::workflow_publication_endpoint_runtime::execute_register_workflow_publication_endpoint_request(
        runtime_state,
        config_projection,
        relay_state,
        RegisterWorkflowPublicationEndpointRequest {
            session_id: request.session_id,
            publication_ref: publication_id,
            local_url: local_url.clone(),
            runtime_session_id: Some(publication.session_id().to_string()),
            ttl_ms: None,
        },
        caller_user_id,
    )
    .await?;
    let LocalDaemonResponse::WorkflowPublicationEndpointRegistered { publication, .. } = registered
    else {
        return Err(publication_runtime_error(
            "bind workflow publication deployment",
            "publication endpoint registration returned an unexpected response",
        ));
    };
    Ok(bound_publication_response(
        publication,
        binding,
        Some(local_url),
        process_id,
        false,
    ))
}

pub(crate) async fn reconcile_bound_workflow_publication_runtimes(
    runtime_state: &KernelRuntimeState,
) {
    let candidates = runtime_state
        .owned
        .session_store
        .read()
        .durable_sessions()
        .into_iter()
        .flat_map(|session| {
            session
                .workflow_publications()
                .iter()
                .filter_map(|publication| {
                    publication_runtime_recovery_binding(publication)
                        .map(|binding| (publication.clone(), binding))
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let now_ms = crate::session::unix_epoch_ms();
    for (publication, binding) in candidates {
        let process_key =
            publication_runtime_process_key(publication.session_id(), publication.id());
        match runtime_state
            .owned
            .workflow_publication_runtimes
            .running(&process_key)
            .await
        {
            Ok(Some(_)) => {
                runtime_state
                    .owned
                    .workflow_publication_runtimes
                    .record_recovery_success(&process_key)
                    .await;
                continue;
            }
            Ok(None) => {}
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.publication_runtime",
                    "failed to inspect bound publication runtime",
                    serde_json::json!({
                        "session_id": publication.session_id(),
                        "publication_id": publication.id(),
                        "deployment_id": binding.deployment_id,
                        "error": error.to_string(),
                    }),
                );
                continue;
            }
        }
        if !runtime_state
            .owned
            .workflow_publication_runtimes
            .recovery_due(&process_key, now_ms)
            .await
        {
            continue;
        }
        let port = if is_schedule_only_publication(&publication) {
            None
        } else {
            match reserve_ephemeral_publication_runtime_port() {
                Ok(port) => Some(port),
                Err(error) => {
                    runtime_state
                        .owned
                        .workflow_publication_runtimes
                        .record_recovery_failure(&process_key, now_ms)
                        .await;
                    crate::logging::warn_with_fields(
                        "daemon.publication_runtime",
                        "failed to reserve bound publication runtime port",
                        serde_json::json!({
                            "session_id": publication.session_id(),
                            "publication_id": publication.id(),
                            "deployment_id": binding.deployment_id,
                            "error": error.to_string(),
                        }),
                    );
                    continue;
                }
            }
        };
        let result = start_publication_runtime(
            runtime_state,
            ControlWorkflowPublicationRuntimeRequest {
                session_id: publication.session_id().to_string(),
                publication_ref: publication.id().to_string(),
                action: WorkflowPublicationRuntimeAction::Start,
                host: Some(DEFAULT_PUBLICATION_RUNTIME_HOST.to_string()),
                port,
                kernel_url: None,
            },
            publication.clone(),
            process_key.clone(),
            PublicationRuntimeLaunchContext {
                cloud_deployment_id: Some(binding.deployment_id.clone()),
                expected_package_digest: Some(binding.package_digest.clone()),
                binding: Some(binding.clone()),
            },
        )
        .await;
        match result {
            Ok(_) => {
                runtime_state
                    .owned
                    .workflow_publication_runtimes
                    .record_recovery_success(&process_key)
                    .await;
                crate::logging::info_with_fields(
                    "daemon.publication_runtime",
                    "recovered bound publication runtime",
                    serde_json::json!({
                        "session_id": publication.session_id(),
                        "publication_id": publication.id(),
                        "deployment_id": binding.deployment_id,
                    }),
                );
            }
            Err(error) => {
                runtime_state
                    .owned
                    .workflow_publication_runtimes
                    .record_recovery_failure(&process_key, now_ms)
                    .await;
                crate::logging::warn_with_fields(
                    "daemon.publication_runtime",
                    "failed to recover bound publication runtime",
                    serde_json::json!({
                        "session_id": publication.session_id(),
                        "publication_id": publication.id(),
                        "deployment_id": binding.deployment_id,
                        "error": error.to_string(),
                    }),
                );
            }
        }
    }
}

async fn inspect_publication_runtime(
    runtime_state: &KernelRuntimeState,
    publication: WorkflowPublicationDefinition,
    process_key: String,
) -> Result<LocalDaemonResponse, DaemonError> {
    let running = runtime_state
        .owned
        .workflow_publication_runtimes
        .running(&process_key)
        .await?;
    let local_url = running
        .as_ref()
        .and_then(|process| process.local_url.clone());
    let process_id = running.as_ref().and_then(|process| process.process_id);
    let runtime_snapshot = if let Some(process) = running.as_ref() {
        publication_runtime_status_snapshot(&process.host, process.port)
            .await
            .ok()
    } else {
        None
    };
    let mut publication = if let Some(process) = running.as_ref() {
        mark_publication_runtime_status(
            runtime_state,
            publication.session_id(),
            publication.id(),
            "running",
            Some(local_url.clone()),
            Some(publication_runtime_metadata_preserving_binding(
                &publication,
                serde_json::json!({
                    "kind": "local_runtime",
                    "status": "running",
                    "host": process.host,
                    "port": process.port,
                    "local_url": process.local_url,
                    "process_id": process.process_id,
                    "package_root": serde_json::Value::Null,
                }),
            )),
        )?
    } else if publication.status() == Some("error") {
        publication
    } else {
        mark_publication_runtime_status(
            runtime_state,
            publication.session_id(),
            publication.id(),
            "stopped",
            Some(None),
            Some(stopped_publication_runtime_metadata(&publication, false)),
        )?
    };
    if let Some(snapshot) = runtime_snapshot {
        let latest_run = snapshot
            .get("latest_run")
            .filter(|value| !value.is_null())
            .cloned();
        let recent_runs = snapshot
            .get("recent_runs")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let latest_output = snapshot
            .get("latest_output")
            .filter(|value| !value.is_null())
            .cloned();
        publication = runtime_state
            .owned
            .session_store
            .write()
            .set_workflow_publication_runtime_run_observability(
                publication.session_id(),
                publication.id(),
                latest_run,
                recent_runs,
                latest_output,
            )?;
    }
    let status = publication.status().unwrap_or("stopped").to_string();
    let open_url = running.as_ref().and_then(|_| {
        publication
            .open_url()
            .map(str::to_string)
            .or(local_url.clone())
    });
    let viewer_url = running.as_ref().and_then(|_| {
        publication
            .viewer_url()
            .map(str::to_string)
            .or_else(|| open_url.clone())
            .or(local_url.clone())
    });
    let message = if running.is_some() {
        "publication runtime is running"
    } else {
        "publication runtime process is not running"
    };
    Ok(LocalDaemonResponse::WorkflowPublicationRuntimeControlled {
        publication,
        action: WorkflowPublicationRuntimeAction::Inspect,
        status,
        local_url,
        open_url,
        viewer_url,
        process_id,
        message: Some(message.to_string()),
    })
}

async fn publication_runtime_status_snapshot(
    host: &str,
    port: u16,
) -> Result<serde_json::Value, String> {
    let mut stream = TokioTcpStream::connect((host, port))
        .await
        .map_err(|error| format!("failed to connect to publication status endpoint: {error}"))?;
    let request = format!(
        "GET /.well-known/chariox/publication/status HTTP/1.1\r\nHost: {}:{}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        host, port,
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|error| format!("failed to request publication status: {error}"))?;
    let mut response = Vec::new();
    stream
        .take(4 * 1024 * 1024)
        .read_to_end(&mut response)
        .await
        .map_err(|error| format!("failed to read publication status: {error}"))?;
    let separator = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "publication status response omitted HTTP headers".to_string())?;
    let headers = String::from_utf8_lossy(&response[..separator]);
    if !headers
        .lines()
        .next()
        .is_some_and(|line| line.contains(" 200 "))
    {
        return Err(format!("publication status endpoint returned {headers}"));
    }
    serde_json::from_slice(&response[separator + 4..])
        .map_err(|error| format!("publication status endpoint returned invalid JSON: {error}"))
}

async fn start_publication_runtime(
    runtime_state: &KernelRuntimeState,
    request: ControlWorkflowPublicationRuntimeRequest,
    publication: WorkflowPublicationDefinition,
    process_key: String,
    launch_context: PublicationRuntimeLaunchContext,
) -> Result<LocalDaemonResponse, DaemonError> {
    let launch_context = publication_runtime_launch_context(&publication, launch_context);
    if !runtime_state
        .owned
        .workflow_publication_runtimes
        .claim_launch(&process_key)
        .await
    {
        return Err(publication_runtime_error(
            "start workflow publication runtime",
            "publication runtime launch is already in progress",
        ));
    }
    let result = start_publication_runtime_claimed(
        runtime_state,
        request,
        publication,
        process_key.clone(),
        launch_context,
    )
    .await;
    runtime_state
        .owned
        .workflow_publication_runtimes
        .release_launch(&process_key)
        .await;
    result
}

async fn start_publication_runtime_claimed(
    runtime_state: &KernelRuntimeState,
    request: ControlWorkflowPublicationRuntimeRequest,
    publication: WorkflowPublicationDefinition,
    process_key: String,
    launch_context: PublicationRuntimeLaunchContext,
) -> Result<LocalDaemonResponse, DaemonError> {
    let host = request
        .host
        .as_deref()
        .and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then_some(trimmed)
        })
        .unwrap_or(DEFAULT_PUBLICATION_RUNTIME_HOST)
        .to_string();
    let is_schedule_only = is_schedule_only_publication(&publication);
    let port = publication_runtime_port(request.port, is_schedule_only);
    if let Some(existing) = runtime_state
        .owned
        .workflow_publication_runtimes
        .running(&process_key)
        .await?
    {
        let refreshed = mark_publication_runtime_status(
            runtime_state,
            &request.session_id,
            publication.id(),
            "running",
            None,
            Some(publication_runtime_metadata_preserving_binding(
                &publication,
                serde_json::json!({
                    "kind": "local_runtime",
                    "status": "running",
                    "host": existing.host,
                    "port": existing.port,
                    "local_url": existing.local_url,
                    "process_id": existing.process_id,
                    "package_root": serde_json::Value::Null,
                }),
            )),
        )?;
        let open_url = refreshed.open_url().map(str::to_string);
        let viewer_url = refreshed.viewer_url().map(str::to_string);
        return Ok(LocalDaemonResponse::WorkflowPublicationRuntimeControlled {
            publication: refreshed,
            action: WorkflowPublicationRuntimeAction::Start,
            status: "running".to_string(),
            local_url: existing.local_url,
            open_url,
            viewer_url,
            process_id: existing.process_id,
            message: Some("publication runtime is already running".to_string()),
        });
    }

    let kernel_url = publication_runtime_kernel_url(runtime_state, request.kernel_url.as_deref());
    let package_kernel_url = publication_runtime_package_kernel_url(
        &kernel_url,
        launch_context.expected_package_digest.as_deref(),
    );
    if let Err(error) = validate_publication_runtime_bind_address(&host, port, is_schedule_only) {
        let message = error.to_string();
        let _ = mark_publication_runtime_error(
            runtime_state,
            &request.session_id,
            publication.id(),
            &message,
        );
        return Err(error);
    }
    if let Some(expected) = launch_context.expected_package_digest.as_deref() {
        let package = runtime_state.owned.workflow_export_publication_package(
            crate::local::ExportWorkflowPublicationPackageRequest {
                session_id: request.session_id.clone(),
                publication_ref: publication.id().to_string(),
                kernel_url: package_kernel_url,
                agent_app: None,
                agent_app_assets_dir: None,
            },
        )?;
        let LocalDaemonResponse::WorkflowPublicationPackageExported { package_digest, .. } =
            package
        else {
            return Err(DaemonError::LocalTransport {
                operation: "start workflow publication runtime",
                message: "publication package export returned an unexpected response".to_string(),
            });
        };
        if package_digest != expected {
            return Err(publication_runtime_error(
                "start workflow publication runtime",
                format!(
                    "publication package digest changed before deployment bind: expected {expected}, got {package_digest}"
                ),
            ));
        }
    }
    let local_url = if is_schedule_only {
        None
    } else {
        Some(publication_local_url(&host, port))
    };
    let mut command = Command::new(resolve_chariox_cli_bin()?);
    command
        .arg("serve")
        .arg("source")
        .arg(&request.session_id)
        .arg(publication.id())
        .arg(port.to_string())
        .arg("--host")
        .arg(&host)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command.arg("--kernel-url").arg(&kernel_url);
    if let Some(deployment_id) = launch_context.cloud_deployment_id.as_deref() {
        command.arg("--cloud-deployment").arg(deployment_id);
    }
    let caller_claims_config = launch_context
        .binding
        .as_ref()
        .map(write_publication_caller_claims_config)
        .transpose()?;
    if let Some(path) = caller_claims_config.as_ref() {
        command.env("CHARIOX_PUBLICATION_CALLER_CLAIMS_CONFIG_FILE", path);
    }
    let mut child = command.spawn().map_err(|error| {
        if let Some(path) = caller_claims_config.as_ref() {
            let _ = fs::remove_file(path);
        }
        let message = format!("failed to launch chariox publication gateway: {error}");
        let _ = mark_publication_runtime_error(
            runtime_state,
            &request.session_id,
            publication.id(),
            &message,
        );
        DaemonError::LocalTransport {
            operation: "start workflow publication runtime",
            message,
        }
    })?;
    let process_id = child.id();
    if let Err(message) =
        wait_for_publication_runtime_start(&mut child, &host, port, is_schedule_only).await
    {
        if let Some(path) = caller_claims_config.as_ref() {
            let _ = fs::remove_file(path);
        }
        let _ = mark_publication_runtime_error(
            runtime_state,
            &request.session_id,
            publication.id(),
            &message,
        );
        return Err(DaemonError::LocalTransport {
            operation: "start workflow publication runtime",
            message,
        });
    }
    if let Some(path) = caller_claims_config.as_ref() {
        let _ = fs::remove_file(path);
    }
    runtime_state
        .owned
        .workflow_publication_runtimes
        .insert(
            process_key,
            WorkflowPublicationRuntimeProcess {
                child,
                process_id,
                host: host.clone(),
                port,
                local_url: local_url.clone(),
            },
        )
        .await;
    let runtime_status = launched_publication_runtime_status(is_schedule_only);
    let publication = mark_publication_runtime_status(
        runtime_state,
        &request.session_id,
        publication.id(),
        runtime_status,
        Some(local_url.clone()),
        Some(publication_runtime_deployment_metadata(
            runtime_status,
            &host,
            port,
            local_url.as_deref(),
            &kernel_url,
            process_id,
            launch_context.binding.as_ref(),
        )),
    )?;
    Ok(LocalDaemonResponse::WorkflowPublicationRuntimeControlled {
        publication,
        action: request.action,
        status: runtime_status.to_string(),
        local_url: local_url.clone(),
        open_url: local_url.clone(),
        viewer_url: local_url,
        process_id,
        message: Some(launched_publication_runtime_message(is_schedule_only).to_string()),
    })
}

fn write_publication_caller_claims_config(
    binding: &WorkflowPublicationDeploymentBinding,
) -> Result<PathBuf, DaemonError> {
    let payload = serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "deployment_id": binding.deployment_id,
        "environment_id": binding.environment_id,
        "public_key_pem": binding.caller_claims_public_key_pem,
    }))
    .map_err(|error| {
        publication_runtime_error(
            "start workflow publication runtime",
            format!("failed to serialize caller claims config: {error}"),
        )
    })?;
    for _ in 0..8 {
        let mut suffix = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut suffix);
        let path = std::env::temp_dir().join(format!(
            "chariox-publication-caller-claims-{}.json",
            suffix
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        match options.open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(&payload).and_then(|_| file.sync_all()) {
                    let _ = fs::remove_file(&path);
                    return Err(publication_runtime_error(
                        "start workflow publication runtime",
                        format!("failed to write caller claims config: {error}"),
                    ));
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(publication_runtime_error(
                    "start workflow publication runtime",
                    format!("failed to create caller claims config: {error}"),
                ));
            }
        }
    }
    Err(publication_runtime_error(
        "start workflow publication runtime",
        "failed to allocate caller claims config",
    ))
}

fn validate_caller_claims_public_key(value: &str) -> Result<(), DaemonError> {
    let lines = value.lines().collect::<Vec<_>>();
    let decoded = lines.get(1).and_then(|encoded| {
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()
            .filter(|der| {
                der.len() == 44
                    && der[..12]
                        == [
                            0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
                        ]
                    && base64::engine::general_purpose::STANDARD.encode(der) == *encoded
            })
    });
    let valid = !value.contains(['\r', '\0'])
        && value.ends_with('\n')
        && lines.len() == 3
        && lines[0] == "-----BEGIN PUBLIC KEY-----"
        && decoded.is_some()
        && lines[2] == "-----END PUBLIC KEY-----";
    if valid {
        return Ok(());
    }
    Err(publication_runtime_error(
        "bind workflow publication deployment",
        "deployment bind caller_claims_public_key_pem must be a canonical Ed25519 SPKI public key",
    ))
}

fn validated_deployment_binding(
    request: &BindWorkflowPublicationDeploymentRequest,
) -> Result<WorkflowPublicationDeploymentBinding, DaemonError> {
    for (label, value) in [
        ("setup_id", request.setup_id.as_str()),
        ("operation_key", request.operation_key.as_str()),
        ("deployment_id", request.deployment_id.as_str()),
        ("environment_id", request.environment_id.as_str()),
        ("release_id", request.release_id.as_str()),
    ] {
        if value.trim().is_empty() || value.len() > 200 || value.contains(['\r', '\n', '\0']) {
            return Err(publication_runtime_error(
                "bind workflow publication deployment",
                format!("deployment bind {label} is invalid"),
            ));
        }
    }
    let expected_operation_key = format!("deployment-setup:{}:runtime", request.setup_id);
    if request.operation_key != expected_operation_key {
        return Err(publication_runtime_error(
            "bind workflow publication deployment",
            format!("deployment bind operation_key must be {expected_operation_key}"),
        ));
    }
    let digest = request
        .package_digest
        .strip_prefix("sha256:")
        .filter(|value| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        });
    if digest.is_none() {
        return Err(publication_runtime_error(
            "bind workflow publication deployment",
            "deployment bind package_digest must be a lowercase sha256 digest",
        ));
    }
    validate_caller_claims_public_key(&request.caller_claims_public_key_pem)?;
    Ok(WorkflowPublicationDeploymentBinding {
        setup_id: request.setup_id.clone(),
        operation_key: request.operation_key.clone(),
        deployment_id: request.deployment_id.clone(),
        environment_id: request.environment_id.clone(),
        release_id: request.release_id.clone(),
        package_digest: request.package_digest.clone(),
        desired_revision: request.desired_revision,
        caller_claims_public_key_pem: request.caller_claims_public_key_pem.clone(),
    })
}

fn publication_deployment_binding(
    publication: &WorkflowPublicationDefinition,
) -> Option<WorkflowPublicationDeploymentBinding> {
    let binding = publication.deployment()?.get("binding")?;
    Some(WorkflowPublicationDeploymentBinding {
        setup_id: binding.get("setup_id")?.as_str()?.to_string(),
        operation_key: binding.get("operation_key")?.as_str()?.to_string(),
        deployment_id: binding.get("deployment_id")?.as_str()?.to_string(),
        environment_id: binding.get("environment_id")?.as_str()?.to_string(),
        release_id: binding.get("release_id")?.as_str()?.to_string(),
        package_digest: binding.get("package_digest")?.as_str()?.to_string(),
        desired_revision: binding.get("desired_revision")?.as_u64()?,
        caller_claims_public_key_pem: binding
            .get("caller_claims_public_key_pem")?
            .as_str()?
            .to_string(),
    })
}

fn publication_runtime_recovery_binding(
    publication: &WorkflowPublicationDefinition,
) -> Option<WorkflowPublicationDeploymentBinding> {
    if !publication.enabled()
        || publication
            .deployment()
            .and_then(|deployment| deployment.get("desired_state"))
            .and_then(serde_json::Value::as_str)
            == Some("stopped")
    {
        return None;
    }
    publication_deployment_binding(publication)
}

fn publication_runtime_launch_context(
    publication: &WorkflowPublicationDefinition,
    mut launch_context: PublicationRuntimeLaunchContext,
) -> PublicationRuntimeLaunchContext {
    if launch_context.binding.is_some() {
        return launch_context;
    }
    let Some(binding) = publication_deployment_binding(publication) else {
        return launch_context;
    };
    if launch_context.cloud_deployment_id.is_none() {
        launch_context.cloud_deployment_id = Some(binding.deployment_id.clone());
    }
    if launch_context.expected_package_digest.is_none() {
        launch_context.expected_package_digest = Some(binding.package_digest.clone());
    }
    launch_context.binding = Some(binding);
    launch_context
}

#[allow(clippy::too_many_arguments)]
fn publication_runtime_deployment_metadata(
    status: &str,
    host: &str,
    port: u16,
    local_url: Option<&str>,
    kernel_url: &str,
    process_id: Option<u32>,
    binding: Option<&WorkflowPublicationDeploymentBinding>,
) -> serde_json::Value {
    let mut deployment = serde_json::json!({
        "kind": "local_runtime",
        "status": status,
        "host": host,
        "port": port,
        "local_url": local_url,
        "kernel_url": kernel_url,
        "process_id": process_id,
        "package_root": serde_json::Value::Null,
    });
    if let Some(binding) = binding {
        deployment["binding"] = serde_json::json!({
            "setup_id": binding.setup_id,
            "operation_key": binding.operation_key,
            "deployment_id": binding.deployment_id,
            "environment_id": binding.environment_id,
            "release_id": binding.release_id,
            "package_digest": binding.package_digest,
            "desired_revision": binding.desired_revision,
            "caller_claims_public_key_pem": binding.caller_claims_public_key_pem,
            "bound_at_ms": crate::session::unix_epoch_ms(),
        });
    }
    deployment
}

fn bound_publication_response(
    publication: WorkflowPublicationDefinition,
    binding: WorkflowPublicationDeploymentBinding,
    local_url: Option<String>,
    process_id: Option<u32>,
    replayed: bool,
) -> LocalDaemonResponse {
    let tunnel_url = publication
        .deployment()
        .filter(|deployment| {
            deployment.get("kind").and_then(serde_json::Value::as_str) == Some("tunnel")
        })
        .and_then(|_| publication.open_url())
        .map(str::to_string);
    let state = if local_url.is_none() {
        "running"
    } else if tunnel_url.is_some() {
        "running"
    } else {
        "waiting_for_relay"
    };
    LocalDaemonResponse::WorkflowPublicationDeploymentBound {
        runtime_session_id: Some(publication.session_id().to_string()),
        publication: Box::new(publication),
        operation_key: binding.operation_key,
        deployment_id: binding.deployment_id,
        release_id: binding.release_id,
        package_digest: binding.package_digest,
        desired_revision: binding.desired_revision,
        state: state.to_string(),
        local_url,
        tunnel_url,
        process_id,
        replayed,
    }
}

fn reserve_ephemeral_publication_runtime_port() -> Result<u16, DaemonError> {
    let listener = TcpListener::bind((DEFAULT_PUBLICATION_RUNTIME_HOST, 0)).map_err(|error| {
        publication_runtime_error(
            "bind workflow publication deployment",
            format!("failed to reserve an ephemeral publication runtime port: {error}"),
        )
    })?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| {
            publication_runtime_error(
                "bind workflow publication deployment",
                format!("failed to inspect the reserved publication runtime port: {error}"),
            )
        })
}

fn publication_runtime_error(operation: &'static str, message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: message.into(),
    }
}

async fn wait_for_publication_runtime_start(
    child: &mut Child,
    host: &str,
    port: u16,
    is_schedule_only: bool,
) -> Result<(), String> {
    let deadline = Instant::now() + PUBLICATION_RUNTIME_START_TIMEOUT;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("failed to inspect launched publication gateway: {error}"))?
        {
            let stderr = publication_runtime_stderr(child).await;
            return Err(format!(
                "publication gateway exited before becoming ready with status {status}{}",
                stderr_suffix(&stderr),
            ));
        }
        if is_schedule_only || TcpStream::connect((host, port)).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill().await;
            let stderr = publication_runtime_stderr(child).await;
            return Err(format!(
                "publication gateway did not listen on {host}:{port} within {}s{}",
                PUBLICATION_RUNTIME_START_TIMEOUT.as_secs(),
                stderr_suffix(&stderr),
            ));
        }
        sleep(PUBLICATION_RUNTIME_START_POLL).await;
    }
}

async fn publication_runtime_stderr(child: &mut Child) -> String {
    let Some(mut stderr) = child.stderr.take() else {
        return String::new();
    };
    let mut output = String::new();
    let _ = stderr.read_to_string(&mut output).await;
    output.trim().to_string()
}

fn stderr_suffix(stderr: &str) -> String {
    if stderr.is_empty() {
        String::new()
    } else {
        format!(": {stderr}")
    }
}

async fn stop_publication_runtime(
    runtime_state: &KernelRuntimeState,
    process_key: &str,
) -> Result<(), DaemonError> {
    let Some(mut process) = runtime_state
        .owned
        .workflow_publication_runtimes
        .remove(process_key)
        .await
    else {
        return Ok(());
    };
    if process
        .child
        .try_wait()
        .map_err(|error| {
            let message = format!("failed to inspect publication gateway: {error}");
            DaemonError::LocalTransport {
                operation: "stop workflow publication runtime",
                message,
            }
        })?
        .is_none()
    {
        process.child.kill().await.map_err(|error| {
            let message = format!("failed to stop publication gateway: {error}");
            DaemonError::LocalTransport {
                operation: "stop workflow publication runtime",
                message,
            }
        })?;
    }
    Ok(())
}

fn mark_publication_runtime_status(
    runtime_state: &KernelRuntimeState,
    session_id: &str,
    publication_ref: &str,
    status: &str,
    open_url: Option<Option<String>>,
    deployment: Option<serde_json::Value>,
) -> Result<WorkflowPublicationDefinition, DaemonError> {
    runtime_state
        .owned
        .session_store
        .write()
        .mark_workflow_publication_runtime_status(
            session_id,
            publication_ref,
            status,
            open_url,
            deployment,
        )
}

fn publication_runtime_metadata_preserving_binding(
    publication: &WorkflowPublicationDefinition,
    mut metadata: serde_json::Value,
) -> serde_json::Value {
    let Some(binding) = publication
        .deployment()
        .and_then(|deployment| deployment.get("binding"))
        .cloned()
    else {
        return metadata;
    };
    if let Some(object) = metadata.as_object_mut() {
        object.insert("binding".to_string(), binding);
    }
    metadata
}

fn stopped_publication_runtime_metadata(
    publication: &WorkflowPublicationDefinition,
    explicitly_stopped: bool,
) -> serde_json::Value {
    let preserve_stopped_intent = explicitly_stopped
        || publication
            .deployment()
            .and_then(|deployment| deployment.get("desired_state"))
            .and_then(serde_json::Value::as_str)
            == Some("stopped");
    let mut metadata = publication_runtime_metadata_preserving_binding(
        publication,
        serde_json::json!({
            "kind": "local_runtime",
            "status": "stopped",
        }),
    );
    if preserve_stopped_intent {
        metadata["desired_state"] = serde_json::json!("stopped");
    }
    metadata
}

fn mark_publication_runtime_error(
    runtime_state: &KernelRuntimeState,
    session_id: &str,
    publication_ref: &str,
    message: &str,
) -> Result<WorkflowPublicationDefinition, DaemonError> {
    runtime_state
        .owned
        .session_store
        .write()
        .mark_workflow_publication_runtime_error(session_id, publication_ref, message)
}

fn publication_runtime_kernel_url(
    runtime_state: &KernelRuntimeState,
    requested_kernel_url: Option<&str>,
) -> String {
    requested_kernel_url
        .and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then_some(trimmed.to_string())
        })
        .unwrap_or_else(|| {
            runtime_state
                .owned
                .config_projection
                .snapshot()
                .kernel_websocket_url()
        })
}

fn publication_runtime_package_kernel_url(
    kernel_url: &str,
    expected_package_digest: Option<&str>,
) -> Option<String> {
    expected_package_digest
        .is_none()
        .then(|| kernel_url.to_string())
}

fn resolve_chariox_cli_bin() -> Result<PathBuf, DaemonError> {
    if let Some(value) = std::env::var_os("CHARIOX_CLI_BIN") {
        return Ok(PathBuf::from(value));
    }
    let current = std::env::current_exe().map_err(|error| DaemonError::LocalTransport {
        operation: "start workflow publication runtime",
        message: format!("failed to resolve current kernel executable: {error}"),
    })?;
    let dir = current
        .parent()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "start workflow publication runtime",
            message: format!(
                "failed to resolve executable directory for `{}`",
                current.display()
            ),
        })?;
    let candidate = dir.join(if cfg!(windows) {
        "chariox-cli.exe"
    } else {
        "chariox-cli"
    });
    if candidate.exists() {
        return Ok(candidate);
    }
    Err(DaemonError::LocalTransport {
        operation: "start workflow publication runtime",
        message: format!(
            "chariox-cli was not found beside `{}`; set CHARIOX_CLI_BIN to enable publication runtime lifecycle",
            current.display()
        ),
    })
}

fn publication_runtime_process_key(session_id: &str, publication_id: &str) -> String {
    format!("{session_id}:{publication_id}")
}

fn publication_local_url(host: &str, port: u16) -> String {
    format!("http://{}:{}/", host, port)
}

fn is_schedule_only_publication(publication: &WorkflowPublicationDefinition) -> bool {
    publication.kind() == crate::session::WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY
}

fn publication_runtime_port(requested_port: Option<u16>, is_schedule_only: bool) -> u16 {
    if is_schedule_only {
        0
    } else {
        requested_port.unwrap_or(DEFAULT_PUBLICATION_RUNTIME_PORT)
    }
}

fn validate_publication_runtime_bind_address(
    host: &str,
    port: u16,
    is_schedule_only: bool,
) -> Result<(), DaemonError> {
    if is_schedule_only {
        return Ok(());
    }
    if port == 0 {
        return Err(DaemonError::LocalTransport {
            operation: "start workflow publication runtime",
            message: "ingress publication runtime port must be between 1 and 65535".to_string(),
        });
    }
    TcpListener::bind((host, port))
        .map(|listener| drop(listener))
        .map_err(|error| DaemonError::LocalTransport {
            operation: "start workflow publication runtime",
            message: format!("publication runtime port {port} is not available on {host}: {error}"),
        })
}

fn launched_publication_runtime_status(is_schedule_only: bool) -> &'static str {
    if is_schedule_only {
        "running"
    } else {
        "starting"
    }
}

fn launched_publication_runtime_message(is_schedule_only: bool) -> &'static str {
    if is_schedule_only {
        "schedule-only publication runtime running; no ingress endpoint is exposed"
    } else {
        "publication runtime starting; endpoint registration will publish a relay display URL when available"
    }
}

#[derive(Clone)]
struct RunningPublicationRuntime {
    process_id: Option<u32>,
    host: String,
    port: u16,
    local_url: Option<String>,
}

impl WorkflowPublicationRuntimeProcessStore {
    async fn claim_launch(&self, key: &str) -> bool {
        self.launching.lock().await.insert(key.to_string())
    }

    async fn release_launch(&self, key: &str) {
        self.launching.lock().await.remove(key);
    }

    async fn recovery_due(&self, key: &str, now_ms: u64) -> bool {
        self.recoveries
            .lock()
            .await
            .get(key)
            .map_or(true, |recovery| recovery.next_attempt_at_ms <= now_ms)
    }

    async fn record_recovery_success(&self, key: &str) {
        self.recoveries.lock().await.remove(key);
    }

    async fn record_recovery_failure(&self, key: &str, now_ms: u64) {
        let mut guard = self.recoveries.lock().await;
        let recovery = guard.entry(key.to_string()).or_default();
        recovery.failures = recovery.failures.saturating_add(1);
        let exponent = recovery.failures.saturating_sub(1).min(6);
        let delay = PUBLICATION_RUNTIME_RECOVERY_BASE_DELAY_MS
            .saturating_mul(1_u64 << exponent)
            .min(PUBLICATION_RUNTIME_RECOVERY_MAX_DELAY_MS);
        recovery.next_attempt_at_ms = now_ms.saturating_add(delay);
    }

    async fn running(&self, key: &str) -> Result<Option<RunningPublicationRuntime>, DaemonError> {
        let mut guard = self.inner.lock().await;
        let Some(process) = guard.get_mut(key) else {
            return Ok(None);
        };
        if process
            .child
            .try_wait()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "inspect workflow publication runtime",
                message: format!("failed to inspect publication gateway: {error}"),
            })?
            .is_some()
        {
            guard.remove(key);
            return Ok(None);
        }
        Ok(Some(RunningPublicationRuntime {
            process_id: process.process_id,
            host: process.host.clone(),
            port: process.port,
            local_url: process.local_url.clone(),
        }))
    }

    async fn insert(&self, key: String, process: WorkflowPublicationRuntimeProcess) {
        self.inner.lock().await.insert(key, process);
    }

    async fn remove(&self, key: &str) -> Option<WorkflowPublicationRuntimeProcess> {
        self.inner.lock().await.remove(key)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        launched_publication_runtime_message, launched_publication_runtime_status,
        publication_local_url, publication_runtime_launch_context,
        publication_runtime_metadata_preserving_binding, publication_runtime_port,
        publication_runtime_recovery_binding, stopped_publication_runtime_metadata,
        validate_publication_runtime_bind_address, validated_deployment_binding,
        write_publication_caller_claims_config, PublicationRuntimeLaunchContext,
        WorkflowPublicationRuntimeProcessStore, DEFAULT_PUBLICATION_RUNTIME_PORT,
    };
    use crate::local::BindWorkflowPublicationDeploymentRequest;
    use std::fs;
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn launched_ingress_runtime_waits_for_endpoint_registration() {
        assert_eq!(launched_publication_runtime_status(false), "starting");
        assert!(launched_publication_runtime_message(false).contains("endpoint registration"));
    }

    #[test]
    fn launched_schedule_only_runtime_is_running_without_ingress_registration() {
        assert_eq!(launched_publication_runtime_status(true), "running");
        assert!(launched_publication_runtime_message(true).contains("no ingress endpoint"));
    }

    #[test]
    fn ingress_runtime_port_uses_requested_or_default_port() {
        assert_eq!(publication_runtime_port(Some(43123), false), 43123);
        assert_eq!(
            publication_runtime_port(None, false),
            DEFAULT_PUBLICATION_RUNTIME_PORT
        );
    }

    #[test]
    fn ingress_runtime_local_url_points_to_gateway_root() {
        assert_eq!(
            publication_local_url("127.0.0.1", 43123),
            "http://127.0.0.1:43123/"
        );
    }

    #[test]
    fn schedule_only_runtime_port_is_ephemeral_internal_port() {
        assert_eq!(publication_runtime_port(Some(43123), true), 0);
        assert_eq!(publication_runtime_port(None, true), 0);
    }

    #[test]
    fn ingress_runtime_rejects_zero_port() {
        let error = validate_publication_runtime_bind_address("127.0.0.1", 0, false)
            .expect_err("ingress port 0 should be rejected");
        assert!(error
            .to_string()
            .contains("port must be between 1 and 65535"));
    }

    #[test]
    fn ingress_runtime_rejects_occupied_port() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("test listener should bind");
        let port = listener.local_addr().expect("local addr").port();
        let error = validate_publication_runtime_bind_address("127.0.0.1", port, false)
            .expect_err("occupied ingress port should be rejected");
        assert!(error.to_string().contains("not available"));
        assert!(error.to_string().contains(&port.to_string()));
    }

    #[test]
    fn schedule_only_runtime_skips_ingress_port_check() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("test listener should bind");
        let port = listener.local_addr().expect("local addr").port();
        validate_publication_runtime_bind_address("127.0.0.1", port, true)
            .expect("schedule-only runtime has no ingress bind");
    }

    #[test]
    fn deployment_binding_requires_setup_scoped_idempotency_and_digest() {
        let mut request = BindWorkflowPublicationDeploymentRequest {
            session_id: "session-1".to_string(),
            publication_ref: "publication-1".to_string(),
            setup_id: "setup-1".to_string(),
            operation_key: "deployment-setup:setup-1:runtime".to_string(),
            deployment_id: "deployment-1".to_string(),
            environment_id: "environment-1".to_string(),
            release_id: "release-1".to_string(),
            package_digest: format!("sha256:{}", "a".repeat(64)),
            desired_revision: 7,
            caller_claims_public_key_pem: "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA/pMgE2dD4Y9eL57S6f9+lve+T2A4M0ueD5GmOZfHjkI=\n-----END PUBLIC KEY-----\n".to_string(),
        };
        let binding = validated_deployment_binding(&request).expect("valid binding should pass");
        let config_path = write_publication_caller_claims_config(&binding)
            .expect("public verifier config should be created");
        let metadata = fs::metadata(&config_path).expect("public verifier config metadata");
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        let config = fs::read_to_string(&config_path).expect("public verifier config");
        assert!(config.contains("\"deployment_id\":\"deployment-1\""));
        assert!(config.contains("\"environment_id\":\"environment-1\""));
        assert!(config.contains("\"public_key_pem\""));
        assert!(!config.contains("secret"));
        fs::remove_file(config_path).expect("public verifier config should be removable");

        request.operation_key = "deployment-setup:other:runtime".to_string();
        assert!(validated_deployment_binding(&request)
            .expect_err("foreign setup key should fail")
            .to_string()
            .contains("operation_key"));
        request.operation_key = "deployment-setup:setup-1:runtime".to_string();
        request.package_digest = "sha256:ABC".to_string();
        assert!(validated_deployment_binding(&request)
            .expect_err("malformed digest should fail")
            .to_string()
            .contains("lowercase sha256"));
        request.package_digest = format!("sha256:{}", "a".repeat(64));
        request.caller_claims_public_key_pem = "not-a-public-key".to_string();
        assert!(validated_deployment_binding(&request)
            .expect_err("non-Ed25519 public key should fail")
            .to_string()
            .contains("canonical Ed25519"));
    }

    #[test]
    fn bound_deployment_digest_validation_uses_a_portable_kernel_url() {
        assert_eq!(
            super::publication_runtime_package_kernel_url(
                "ws://127.0.0.1:43118",
                Some(&format!("sha256:{}", "a".repeat(64))),
            ),
            None,
        );
    }

    #[test]
    fn direct_runtime_digest_validation_keeps_its_requested_kernel_url() {
        assert_eq!(
            super::publication_runtime_package_kernel_url("ws://127.0.0.1:43118", None),
            Some("ws://127.0.0.1:43118".to_string()),
        );
    }

    #[test]
    fn runtime_status_metadata_preserves_the_durable_cloud_binding() {
        let publication = crate::session::WorkflowPublicationDefinition::new(
            "publication-1",
            "session-1",
            "workflow-1",
            "endpoint-1",
            None,
            Some("published".to_string()),
            "ingress",
            Some("/".to_string()),
            vec!["GET".to_string()],
            None,
            None,
            None,
            None,
            Some("async".to_string()),
            None,
            None,
            "owner-1",
        );
        let mut publication = publication;
        publication.mark_served(
            "running",
            "https://relay.example.test/display/publication-1/",
            serde_json::json!({
                "kind": "tunnel",
                "expires_at_ms": 123,
                "binding": {
                    "setup_id": "setup-1",
                    "operation_key": "deployment-setup:setup-1:runtime",
                    "deployment_id": "deployment-1",
                    "environment_id": "environment-1",
                    "release_id": "release-1",
                    "package_digest": format!("sha256:{}", "a".repeat(64)),
                    "desired_revision": 7,
                    "caller_claims_public_key_pem": "public-key",
                },
            }),
        );

        let metadata = stopped_publication_runtime_metadata(&publication, false);

        assert_eq!(metadata["kind"], "local_runtime");
        assert_eq!(metadata["status"], "stopped");
        assert_eq!(
            metadata.pointer("/binding/deployment_id"),
            Some(&serde_json::json!("deployment-1")),
        );
        assert!(metadata.get("expires_at_ms").is_none());
        assert!(metadata.get("desired_state").is_none());
        assert!(publication_runtime_recovery_binding(&publication).is_some());

        let explicit_stop_metadata = stopped_publication_runtime_metadata(&publication, true);
        assert_eq!(
            explicit_stop_metadata.get("desired_state"),
            Some(&serde_json::json!("stopped")),
        );
        publication.mark_served("stopped", "", explicit_stop_metadata);
        assert!(publication_runtime_recovery_binding(&publication).is_none());
        assert_eq!(
            stopped_publication_runtime_metadata(&publication, false).get("desired_state"),
            Some(&serde_json::json!("stopped")),
        );
        let restarted_metadata = publication_runtime_metadata_preserving_binding(
            &publication,
            serde_json::json!({
                "kind": "local_runtime",
                "status": "running",
            }),
        );
        assert!(restarted_metadata.get("desired_state").is_none());
        assert!(restarted_metadata.get("binding").is_some());
        let launch_context = publication_runtime_launch_context(
            &publication,
            PublicationRuntimeLaunchContext::default(),
        );
        assert_eq!(
            launch_context.cloud_deployment_id.as_deref(),
            Some("deployment-1"),
        );
        let expected_package_digest = format!("sha256:{}", "a".repeat(64));
        assert_eq!(
            launch_context.expected_package_digest.as_deref(),
            Some(expected_package_digest.as_str()),
        );
        assert_eq!(
            launch_context
                .binding
                .as_ref()
                .map(|binding| binding.setup_id.as_str()),
            Some("setup-1"),
        );

        let unbound_publication = crate::session::WorkflowPublicationDefinition::new(
            "publication-2",
            "session-1",
            "workflow-1",
            "endpoint-1",
            None,
            Some("published".to_string()),
            "ingress",
            Some("/".to_string()),
            vec!["GET".to_string()],
            None,
            None,
            None,
            None,
            Some("async".to_string()),
            None,
            None,
            "owner-1",
        );
        assert_eq!(
            stopped_publication_runtime_metadata(&unbound_publication, false),
            serde_json::json!({
                "kind": "local_runtime",
                "status": "stopped",
            }),
        );
    }

    #[tokio::test]
    async fn deployment_runtime_recovery_uses_bounded_exponential_backoff() {
        let store = WorkflowPublicationRuntimeProcessStore::default();
        assert!(store.recovery_due("publication-1", 100).await);
        store.record_recovery_failure("publication-1", 100).await;
        assert!(!store.recovery_due("publication-1", 1_099).await);
        assert!(store.recovery_due("publication-1", 1_100).await);
        store.record_recovery_failure("publication-1", 1_100).await;
        assert!(!store.recovery_due("publication-1", 3_099).await);
        assert!(store.recovery_due("publication-1", 3_100).await);
        store.record_recovery_success("publication-1").await;
        assert!(store.recovery_due("publication-1", 3_100).await);
    }
}
