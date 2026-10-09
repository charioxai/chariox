//! Kernel-owned lifecycle control for local workflow publication runtimes.

use super::workflow_publication_owned_state::ExportAppPlan;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
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
/// A recovered runtime that stays up this long clears its backoff.
const PUBLICATION_RUNTIME_RECOVERY_STABLE_MS: u64 = 60_000;
// A source checkout may need to build the TypeScript gateway before it can
// listen. Keep the readiness deadline long enough for that one-time build;
// subsequent launches remain effectively immediate.
const PUBLICATION_RUNTIME_START_TIMEOUT: Duration = Duration::from_secs(60);
const PUBLICATION_RUNTIME_START_POLL: Duration = Duration::from_millis(50);
const PUBLICATION_GATEWAY_PARENT_PIPE_ENV: &str = "CHARIOX_PUBLICATION_EXIT_ON_STDIN_CLOSE";
/// P1.20: the runtime keys of App-bound deployment copies.
pub(super) const DEPLOYMENT_COPY_KEY_PREFIX: &str = "deployment:";
const PACKAGE_DIGEST_MISMATCH: &str =
    "publication package digest no longer matches the bound deployment";

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
    /// A package digest mismatch cannot heal by retrying: recovery waits until
    /// the deployment is rebound to a different package digest.
    parked_for_digest: Option<String>,
    /// When recovery last launched the runtime.
    launched_at_ms: Option<u64>,
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
            let copied = publication_deployment_binding(&publication)
                .filter(|_| publication.apps().is_some());
            let publication = mark_publication_runtime_status(
                runtime_state,
                &request.session_id,
                &publication_id,
                "stopped",
                Some(None),
                Some(stopped_publication_runtime_metadata(&publication, true)),
            )?;
            // The stop is durable first: a restart never serves it again.
            persist_publication_deployment(runtime_state, &request.session_id)?;
            // A stopped App-bound deployment keeps no copy; the owner's
            // routes it had taken over resume. The stop stands if removal
            // fails for now: the runtime reconcile removes the copy of a
            // stopped source.
            if let Some(binding) = copied {
                if let Err(error) = remove_deployment_app_copy(
                    runtime_state,
                    publication.created_by_user_id(),
                    &binding.deployment_id,
                )
                .await
                {
                    crate::logging::warn_with_fields(
                        "daemon.publication_runtime",
                        "a stopped deployment's copy will be removed by the reconcile",
                        serde_json::json!({
                            "deployment_id": binding.deployment_id,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
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
            let publication =
                persist_start_intent(runtime_state, &request.session_id, publication)?;
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
                persist_publication_deployment(runtime_state, &request.session_id)?;
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

    // Verify the release before stopping the one that runs (a rollback after
    // the source changed is refused): a refused bind leaves it serving.
    let digests = bound_release_package_digests(
        runtime_state,
        &request.session_id,
        &publication,
        &binding.package_digest,
        None,
    )
    .map_err(|error| match error {
        DaemonError::LocalTransport { message, .. } => {
            publication_runtime_error("bind workflow publication deployment", message)
        }
        other => other,
    })?;
    if let Err(message) = validate_bound_release(&publication, &binding.package_digest, &digests) {
        return Err(publication_runtime_error(
            "bind workflow publication deployment",
            message,
        ));
    }
    // A deploy is an explicit start: it clears an earlier stop.
    let publication = persist_start_intent(runtime_state, &request.session_id, publication)?;
    if runtime_state
        .owned
        .workflow_publication_runtimes
        .running(&process_key)
        .await?
        .is_some()
    {
        stop_publication_runtime(runtime_state, &process_key).await?;
    }
    let port = if !publication_has_ingress(&publication) {
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
            runtime_session_id: Some(publication_runtime_session_id(&publication).to_string()),
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
    persist_publication_deployment(runtime_state, publication.session_id())?;
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
    stop_disabled_publication_runtimes(runtime_state).await;
    let candidates = runtime_state
        .owned
        .session_store
        .read()
        .durable_session_refs()
        .flat_map(|session| {
            session
                .workflow_publications()
                .iter()
                .filter(|publication| !is_deployment_copy(publication))
                .filter_map(|publication| {
                    publication_runtime_recovery_binding(publication)
                        .map(|binding| (publication.clone(), binding))
                })
        })
        .collect::<Vec<_>>();
    remove_orphaned_deployment_copies(runtime_state).await;
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
                    .record_recovery_running(&process_key, now_ms)
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
            .recovery_due(&process_key, now_ms, &binding.package_digest)
            .await
            || runtime_state
                .owned
                .workflow_publication_runtimes
                .launching(&process_key)
                .await
        {
            continue;
        }
        // An App-bound deployment's copy may wait for App installs, which this
        // pump drives: recover it beside the pump.
        if publication.apps().is_some() {
            let state = runtime_state.clone();
            tokio::spawn(async move {
                recover_bound_publication_runtime(&state, publication, binding, process_key, now_ms)
                    .await
            });
            continue;
        }
        recover_bound_publication_runtime(runtime_state, publication, binding, process_key, now_ms)
            .await;
    }
}

async fn recover_bound_publication_runtime(
    runtime_state: &KernelRuntimeState,
    publication: WorkflowPublicationDefinition,
    binding: WorkflowPublicationDeploymentBinding,
    process_key: String,
    now_ms: u64,
) {
    let port = if !publication_has_ingress(&publication) {
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
                return;
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
    // An App-bound start can take seconds: the backoff counts from its end.
    let finished_ms = crate::session::unix_epoch_ms();
    match result {
        Ok(_) => {
            // A gateway can report started and still exit during startup: the
            // launch keeps its backoff until the runtime has stayed up.
            runtime_state
                .owned
                .workflow_publication_runtimes
                .record_recovery_launch(&process_key, finished_ms)
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
        Err(error) if is_package_digest_mismatch(&error) => {
            runtime_state
                .owned
                .workflow_publication_runtimes
                .park_recovery(&process_key, &binding.package_digest)
                .await;
            crate::logging::warn_with_fields(
                "daemon.publication_runtime",
                "bound publication runtime waits for its deployment to be rebound",
                serde_json::json!({
                    "session_id": publication.session_id(),
                    "publication_id": publication.id(),
                    "deployment_id": binding.deployment_id,
                    "error": error.to_string(),
                }),
            );
        }
        Err(error) => {
            runtime_state
                .owned
                .workflow_publication_runtimes
                .record_recovery_failure(&process_key, finished_ms)
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

/// A disabled publication (for example, one whose workflow was deleted) must
/// not keep its gateway serving requests the kernel will refuse.
async fn stop_disabled_publication_runtimes(runtime_state: &KernelRuntimeState) {
    let disabled = runtime_state
        .owned
        .session_store
        .read()
        .durable_sessions()
        .into_iter()
        .flat_map(|session| {
            session
                .workflow_publications()
                .iter()
                .filter(|publication| !publication.enabled())
                .cloned()
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    for publication in disabled {
        let process_key =
            publication_runtime_process_key(publication.session_id(), publication.id());
        match runtime_state
            .owned
            .workflow_publication_runtimes
            .running(&process_key)
            .await
        {
            Ok(Some(_)) => {}
            Ok(None) => continue,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.publication_runtime",
                    "failed to inspect disabled publication runtime",
                    serde_json::json!({
                        "session_id": publication.session_id(),
                        "publication_id": publication.id(),
                        "error": error.to_string(),
                    }),
                );
                continue;
            }
        }
        let result = stop_publication_runtime(runtime_state, &process_key)
            .await
            .and_then(|()| {
                mark_publication_runtime_status(
                    runtime_state,
                    publication.session_id(),
                    publication.id(),
                    "stopped",
                    Some(None),
                    Some(stopped_publication_runtime_metadata(&publication, false)),
                )
            });
        if let Err(error) = result {
            crate::logging::warn_with_fields(
                "daemon.publication_runtime",
                "failed to stop disabled publication runtime",
                serde_json::json!({
                    "session_id": publication.session_id(),
                    "publication_id": publication.id(),
                    "error": error.to_string(),
                }),
            );
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
        "GET /.well-known/chariox/publication/status HTTP/1.1\r\nHost: {host}:{port}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
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
    let no_ingress = !publication_has_ingress(&publication);
    let port = publication_runtime_port(request.port, no_ingress);
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
    if let Err(error) = validate_publication_runtime_bind_address(&host, port, no_ingress) {
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
        let digests = match bound_release_package_digests(
            runtime_state,
            &request.session_id,
            &publication,
            expected,
            package_kernel_url,
        ) {
            Ok(digests) => digests,
            Err(error) => {
                let _ = mark_publication_runtime_error(
                    runtime_state,
                    &request.session_id,
                    publication.id(),
                    &error.to_string(),
                );
                return Err(error);
            }
        };
        if let Err(message) = validate_bound_release(&publication, expected, &digests) {
            let _ = mark_publication_runtime_error(
                runtime_state,
                &request.session_id,
                publication.id(),
                &message,
            );
            return Err(publication_runtime_error(
                "start workflow publication runtime",
                message,
            ));
        }
    }
    // P1.20: a bound App-bound publication runs as its deployment's copy.
    let copy_session_id = match launch_context.binding.as_ref() {
        Some(binding) if publication.apps().is_some() => {
            match deployment_app_copy(runtime_state, &publication, binding).await {
                Ok(session_id) => session_id,
                Err(error) => {
                    let _ = mark_publication_runtime_error(
                        runtime_state,
                        &request.session_id,
                        publication.id(),
                        &error.to_string(),
                    );
                    return Err(error);
                }
            }
        }
        _ => None,
    };
    let local_url = if no_ingress {
        None
    } else {
        Some(publication_local_url(&host, port))
    };
    let mut command = Command::new(resolve_chariox_cli_bin()?);
    command
        .arg("serve")
        .arg("source")
        .arg(copy_session_id.as_deref().unwrap_or(&request.session_id))
        .arg(publication.id())
        .arg(port.to_string())
        .arg("--host")
        .arg(&host)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    exit_gateway_with_kernel(&mut command);
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
    if let Err(message) = wait_for_publication_runtime_start(
        &mut child,
        &host,
        port,
        no_ingress,
        caller_claims_config.as_deref(),
    )
    .await
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
    register_launched_runtime(
        runtime_state,
        &request.session_id,
        publication.id(),
        process_key,
        WorkflowPublicationRuntimeProcess {
            child,
            process_id,
            host: host.clone(),
            port,
            local_url: local_url.clone(),
        },
    )
    .await?;
    let runtime_status = launched_publication_runtime_status(no_ingress);
    let mut deployment = publication_runtime_deployment_metadata(
        runtime_status,
        &host,
        port,
        local_url.as_deref(),
        &kernel_url,
        process_id,
        launch_context.binding.as_ref(),
    );
    if let Some(copy_session_id) = copy_session_id.as_deref() {
        deployment["app_copy_session_id"] = serde_json::json!(copy_session_id);
        // The gateway registers the copy's endpoint: the same binding gives it
        // the deployment's stable tunnel.
        mark_publication_runtime_status(
            runtime_state,
            copy_session_id,
            publication.id(),
            runtime_status,
            Some(local_url.clone()),
            Some(deployment.clone()),
        )?;
    }
    let publication = mark_publication_runtime_status(
        runtime_state,
        &request.session_id,
        publication.id(),
        runtime_status,
        Some(local_url.clone()),
        Some(deployment),
    )?;
    // A bound deployment (a bind or a recovery) survives a restart, with its
    // copy's binding.
    if launch_context.binding.is_some() {
        persist_publication_deployment(runtime_state, &request.session_id)?;
    }
    Ok(LocalDaemonResponse::WorkflowPublicationRuntimeControlled {
        publication,
        action: request.action,
        status: runtime_status.to_string(),
        local_url: local_url.clone(),
        open_url: local_url.clone(),
        viewer_url: local_url,
        process_id,
        message: Some(launched_publication_runtime_message(no_ingress).to_string()),
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
    if !publication.enabled() || stop_intended(publication) {
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

/// The gateway's stdin is a pipe whose write end the kernel holds in the
/// gateway's `Child` (kept in the runtime store). When the kernel dies for any
/// reason, including SIGKILL, the OS closes it and the gateway exits, so no
/// gateway outlives its kernel. The env tells the gateway its stdin is this
/// pipe; a gateway run by hand keeps its terminal stdin semantics.
fn exit_gateway_with_kernel(command: &mut Command) {
    command
        .stdin(Stdio::piped())
        .env(PUBLICATION_GATEWAY_PARENT_PIPE_ENV, "1");
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
    no_ingress: bool,
    caller_claims_config: Option<&Path>,
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
        // A gateway with no ingress listens for nothing: it has started once it
        // consumed its caller-claims config (a Cloud deployment's), which the
        // kernel must not remove before.
        let ready = if no_ingress {
            caller_claims_config.is_none_or(|path| !path.exists())
        } else {
            TcpStream::connect((host, port)).is_ok()
        };
        if ready {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill().await;
            let stderr = publication_runtime_stderr(child).await;
            let waited_for = if no_ingress {
                "read its deployment config".to_string()
            } else {
                format!("listen on {host}:{port}")
            };
            return Err(format!(
                "publication gateway did not {waited_for} within {}s{}",
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

/// A deployment's binding (or its removal) survives a kernel restart, so
/// recovery serves the deployment again.
/// An App-bound deployment's copy session carries the same binding and is
/// persisted with it.
fn persist_publication_deployment(
    runtime_state: &KernelRuntimeState,
    session_id: &str,
) -> Result<(), DaemonError> {
    let session = runtime_state
        .owned
        .persist_workflow_runtime_session(session_id, "workflow_publication_deployment")?;
    let copies = session
        .workflow_publications()
        .iter()
        .filter_map(|publication| {
            publication
                .deployment()?
                .get("app_copy_session_id")?
                .as_str()
                .map(str::to_owned)
        })
        .collect::<std::collections::BTreeSet<_>>();
    for copy in copies {
        if runtime_state
            .owned
            .session_store
            .read()
            .get_session(&copy)
            .is_ok()
        {
            runtime_state
                .owned
                .persist_workflow_runtime_session(&copy, "workflow_publication_deployment")?;
        }
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
    for key in ["binding", "app_copy_session_id"] {
        let Some(value) = publication
            .deployment()
            .and_then(|deployment| deployment.get(key))
            .cloned()
        else {
            continue;
        };
        if let Some(object) = metadata.as_object_mut() {
            object.insert(key.to_string(), value);
        }
    }
    metadata
}

/// The session a publication's runtime serves: its deployment's copy, if any.
fn publication_runtime_session_id(publication: &WorkflowPublicationDefinition) -> &str {
    publication
        .deployment()
        .and_then(|deployment| deployment.get("app_copy_session_id"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(publication.session_id())
}

/// P1.20: the publication of an App-bound deployment's copy; its runtime is
/// served, bound and recovered through the source publication.
fn is_deployment_copy(publication: &WorkflowPublicationDefinition) -> bool {
    publication
        .runtime_materialization()
        .is_some_and(|materialization| materialization.key.starts_with(DEPLOYMENT_COPY_KEY_PREFIX))
}

#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
async fn deployment_app_copy(
    runtime_state: &KernelRuntimeState,
    publication: &WorkflowPublicationDefinition,
    binding: &WorkflowPublicationDeploymentBinding,
) -> Result<Option<String>, DaemonError> {
    // `None`: the bound release uses no App, so it runs from the source.
    Ok(runtime_state
        .ensure_deployment_app_copy(
            publication,
            &binding.deployment_id,
            &binding.release_id,
            &binding.package_digest,
        )
        .await?
        .map(|copy| copy.session_id))
}

#[cfg(not(any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))))]
async fn deployment_app_copy(
    _runtime_state: &KernelRuntimeState,
    _publication: &WorkflowPublicationDefinition,
    _binding: &WorkflowPublicationDeploymentBinding,
) -> Result<Option<String>, DaemonError> {
    Err(publication_runtime_error(
        "start workflow publication runtime",
        "Apps are not supported on this platform",
    ))
}

/// Copies of App-bound deployments whose source is gone or no longer bound to
/// them are removed; not while an App-bound bind or recovery may be creating
/// one.
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
async fn remove_orphaned_deployment_copies(runtime_state: &KernelRuntimeState) {
    let sources = runtime_state
        .owned
        .session_store
        .read()
        .durable_sessions()
        .into_iter()
        .flat_map(|session| session.workflow_publications().to_vec())
        .filter(|publication| publication.apps().is_some() && !is_deployment_copy(publication))
        .collect::<Vec<_>>();
    let mut live = std::collections::BTreeMap::new();
    let mut retired = Vec::new();
    for publication in sources {
        let process_key =
            publication_runtime_process_key(publication.session_id(), publication.id());
        if runtime_state
            .owned
            .workflow_publication_runtimes
            .launching(&process_key)
            .await
        {
            return;
        }
        let stopped = stop_intended(&publication);
        if publication.enabled() && !stopped {
            live.insert(
                (
                    publication.created_by_user_id().to_owned(),
                    publication.id().to_owned(),
                ),
                publication_deployment_binding(&publication).map(|binding| binding.deployment_id),
            );
        } else {
            retired.push((publication, process_key));
        }
    }
    // A retired source stops serving its copy before the copy goes.
    for (publication, process_key) in retired {
        if let Err(error) =
            stop_retired_publication_runtime(runtime_state, &publication, &process_key).await
        {
            crate::logging::warn_with_fields(
                "daemon.publication_runtime",
                "failed to stop a retired App-bound publication runtime",
                serde_json::json!({
                    "session_id": publication.session_id(),
                    "publication_id": publication.id(),
                    "error": error.to_string(),
                }),
            );
        }
    }
    // A runtime whose source session was deleted serves nothing.
    for process_key in runtime_state
        .owned
        .workflow_publication_runtimes
        .keys()
        .await
    {
        let Some((session_id, _)) = process_key.split_once(':') else {
            continue;
        };
        if runtime_state
            .owned
            .session_store
            .read()
            .get_session(session_id)
            .is_err()
        {
            if let Err(error) = stop_publication_runtime(runtime_state, &process_key).await {
                crate::logging::warn_with_fields(
                    "daemon.publication_runtime",
                    "failed to stop the runtime of a deleted session",
                    serde_json::json!({
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
            }
        }
    }
    runtime_state.remove_orphaned_deployment_copies(&live).await;
}

#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
async fn stop_retired_publication_runtime(
    runtime_state: &KernelRuntimeState,
    publication: &WorkflowPublicationDefinition,
    process_key: &str,
) -> Result<(), DaemonError> {
    if runtime_state
        .owned
        .workflow_publication_runtimes
        .running(process_key)
        .await?
        .is_some()
    {
        stop_publication_runtime(runtime_state, process_key).await?;
    } else if publication.deployment().is_none()
        || (publication.status() == Some("stopped")
            && publication
                .deployment()
                .and_then(|deployment| deployment.get("app_copy_session_id"))
                .is_none())
    {
        return Ok(());
    }
    mark_publication_runtime_status(
        runtime_state,
        publication.session_id(),
        publication.id(),
        "stopped",
        Some(None),
        Some(stopped_publication_runtime_metadata(publication, false)),
    )?;
    persist_publication_deployment(runtime_state, publication.session_id())
}

#[cfg(not(any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))))]
async fn remove_orphaned_deployment_copies(_runtime_state: &KernelRuntimeState) {}

#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
async fn remove_deployment_app_copy(
    runtime_state: &KernelRuntimeState,
    owner: &str,
    deployment_id: &str,
) -> Result<(), DaemonError> {
    runtime_state
        .remove_deployment_app_copy(owner, deployment_id)
        .await
}

#[cfg(not(any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))))]
async fn remove_deployment_app_copy(
    _runtime_state: &KernelRuntimeState,
    _owner: &str,
    _deployment_id: &str,
) -> Result<(), DaemonError> {
    Ok(())
}

/// A user start or a deploy makes its intent durable first, as a stop does:
/// a start that waits for App approvals must not leave the source looking
/// stopped (the reconcile would remove the copies it installed), and a stop
/// that lands after this still wins when the launch registers.
/// Registers a launched gateway, unless a stop landed while it was starting:
/// that stop wins. It could not reach an unregistered process, and writing
/// the running state now would drop the persisted stop (recovery would then
/// serve it again).
async fn register_launched_runtime(
    runtime_state: &KernelRuntimeState,
    session_id: &str,
    publication_id: &str,
    process_key: String,
    mut process: WorkflowPublicationRuntimeProcess,
) -> Result<(), DaemonError> {
    let stopped_meanwhile = runtime_state
        .owned
        .session_store
        .read()
        .resolve_workflow_publication_ref(session_id, publication_id)
        .map(|current| stop_intended(&current))
        .unwrap_or(false);
    if stopped_meanwhile {
        let _ = process.child.kill().await;
        return Err(publication_runtime_error(
            "start workflow publication runtime",
            "the publication was stopped while its runtime was starting",
        ));
    }
    runtime_state
        .owned
        .workflow_publication_runtimes
        .insert(process_key, process)
        .await;
    Ok(())
}

fn persist_start_intent(
    runtime_state: &KernelRuntimeState,
    session_id: &str,
    publication: WorkflowPublicationDefinition,
) -> Result<WorkflowPublicationDefinition, DaemonError> {
    if !stop_intended(&publication) {
        return Ok(publication);
    }
    let publication = mark_publication_runtime_status(
        runtime_state,
        session_id,
        publication.id(),
        "stopped",
        None,
        Some(publication_runtime_metadata_preserving_binding(
            &publication,
            serde_json::json!({ "kind": "local_runtime", "status": "stopped" }),
        )),
    )?;
    persist_publication_deployment(runtime_state, session_id)?;
    Ok(publication)
}

/// Whether the owner stopped the publication's deployment and has not
/// started it again.
fn stop_intended(publication: &WorkflowPublicationDefinition) -> bool {
    publication
        .deployment()
        .and_then(|deployment| deployment.get("desired_state"))
        .and_then(serde_json::Value::as_str)
        == Some("stopped")
}

fn stopped_publication_runtime_metadata(
    publication: &WorkflowPublicationDefinition,
    explicitly_stopped: bool,
) -> serde_json::Value {
    let preserve_stopped_intent = explicitly_stopped || stop_intended(publication);
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
    if let Some(object) = metadata.as_object_mut() {
        object.remove("app_copy_session_id");
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

/// A bound release's re-exported package digest and inputs digest.
pub(super) struct BoundReleaseDigests {
    package: String,
    inputs: String,
}

/// Protocol 377: a bound release's package, re-exported with that release's
/// own App plan and never the owner's current App set, for its digest check.
pub(super) fn bound_release_package_digests(
    runtime_state: &KernelRuntimeState,
    session_id: &str,
    publication: &WorkflowPublicationDefinition,
    expected: &str,
    kernel_url: Option<String>,
) -> Result<BoundReleaseDigests, DaemonError> {
    let release_apps = publication.release_app_plan(expected).cloned();
    let without_apps = publication.release_without_apps(expected);
    if publication.apps().is_some() && release_apps.is_none() && !without_apps {
        return Err(publication_runtime_error(
            "start workflow publication runtime",
            format!(
                "release package {expected} has no App plan recorded on this kernel; export the release again"
            ),
        ));
    }
    match runtime_state.owned.workflow_export_publication_package(
        crate::local::ExportWorkflowPublicationPackageRequest {
            session_id: session_id.to_string(),
            publication_ref: publication.id().to_string(),
            kernel_url,
            agent_app: None,
            agent_app_assets_dir: None,
        },
        match (&release_apps, without_apps) {
            (_, true) => ExportAppPlan::NoApps,
            (Some(plan), false) => ExportAppPlan::Plan(plan),
            (None, false) => ExportAppPlan::Latest,
        },
    )? {
        LocalDaemonResponse::WorkflowPublicationPackageExported {
            package_digest,
            package_files,
            ..
        } => Ok(BoundReleaseDigests {
            package: package_digest,
            inputs:
                super::workflow_publication_owned_state::workflow_publication_release_inputs_digest(
                    &package_files,
                )?,
        }),
        _ => Err(DaemonError::LocalTransport {
            operation: "start workflow publication runtime",
            message: "publication package export returned an unexpected response".to_string(),
        }),
    }
}

#[cfg(test)]
impl KernelRuntimeState {
    /// Whether a bind or recovery of `expected` verifies: the error is the
    /// mismatch it reports.
    pub(crate) fn fixture_verify_bound_release(
        &self,
        session_id: &str,
        publication_id: &str,
        expected: &str,
    ) -> Result<Result<(), String>, DaemonError> {
        let publication = self
            .owned
            .session_store
            .read()
            .resolve_workflow_publication_ref(session_id, publication_id)?;
        let digests =
            bound_release_package_digests(self, session_id, &publication, expected, None)?;
        Ok(validate_bound_release(&publication, expected, &digests))
    }

    /// A launched gateway for this publication: a long-running child.
    pub(crate) async fn fixture_run_publication_runtime(
        &self,
        session_id: &str,
        publication_id: &str,
    ) {
        let child = Command::new("sleep")
            .arg("30")
            .kill_on_drop(true)
            .spawn()
            .expect("runtime child");
        let process_id = child.id();
        self.owned
            .workflow_publication_runtimes
            .insert(
                publication_runtime_process_key(session_id, publication_id),
                WorkflowPublicationRuntimeProcess {
                    child,
                    process_id,
                    host: DEFAULT_PUBLICATION_RUNTIME_HOST.to_string(),
                    port: 0,
                    local_url: None,
                },
            )
            .await;
    }

    pub(crate) async fn fixture_publication_runtime_running(
        &self,
        session_id: &str,
        publication_id: &str,
    ) -> bool {
        self.owned
            .workflow_publication_runtimes
            .running(&publication_runtime_process_key(session_id, publication_id))
            .await
            .ok()
            .flatten()
            .is_some()
    }
}

fn publication_runtime_package_kernel_url(
    kernel_url: &str,
    expected_package_digest: Option<&str>,
) -> Option<String> {
    expected_package_digest
        .is_none()
        .then(|| kernel_url.to_string())
}

/// Protocol 378: a release with a recorded inputs digest verifies by its
/// workflow-owned files, so a kernel upgrade that changes the package's
/// templates keeps it bound; an older release needs its whole package digest.
fn validate_bound_release(
    publication: &WorkflowPublicationDefinition,
    expected: &str,
    actual: &BoundReleaseDigests,
) -> Result<(), String> {
    match publication.release_inputs_digest(expected) {
        Some(recorded) if recorded == actual.inputs => Ok(()),
        Some(recorded) => Err(format!(
            "publication package digest no longer matches the bound deployment: release {expected}'s workflow inputs were {recorded}, now {}; rebind the deployment before restarting",
            actual.inputs
        )),
        None => validate_bound_publication_package_digest(expected, &actual.package),
    }
}

fn validate_bound_publication_package_digest(expected: &str, actual: &str) -> Result<(), String> {
    if actual == expected {
        return Ok(());
    }
    Err(format!(
        "{PACKAGE_DIGEST_MISMATCH}: expected {expected}, got {actual}; rebind the deployment before restarting"
    ))
}

fn is_package_digest_mismatch(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::LocalTransport { message, .. } if message.starts_with(PACKAGE_DIGEST_MISMATCH)
    )
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
    format!("http://{host}:{port}/")
}

/// Schedule-only and App-event triggers take no requests: their gateway gets
/// no port or local URL, and its runtime is running once launched.
fn publication_has_ingress(publication: &WorkflowPublicationDefinition) -> bool {
    !matches!(
        publication.kind(),
        crate::session::WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY
            | crate::session::WORKFLOW_PUBLICATION_KIND_EVENT_BASED
    )
}

fn publication_runtime_port(requested_port: Option<u16>, no_ingress: bool) -> u16 {
    if no_ingress {
        0
    } else {
        requested_port.unwrap_or(DEFAULT_PUBLICATION_RUNTIME_PORT)
    }
}

fn validate_publication_runtime_bind_address(
    host: &str,
    port: u16,
    no_ingress: bool,
) -> Result<(), DaemonError> {
    if no_ingress {
        return Ok(());
    }
    if port == 0 {
        return Err(DaemonError::LocalTransport {
            operation: "start workflow publication runtime",
            message: "ingress publication runtime port must be between 1 and 65535".to_string(),
        });
    }
    TcpListener::bind((host, port))
        .map(drop)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "start workflow publication runtime",
            message: format!("publication runtime port {port} is not available on {host}: {error}"),
        })
}

fn launched_publication_runtime_status(no_ingress: bool) -> &'static str {
    if no_ingress {
        "running"
    } else {
        "starting"
    }
}

fn launched_publication_runtime_message(no_ingress: bool) -> &'static str {
    if no_ingress {
        "publication runtime running; its trigger takes no requests, so no ingress endpoint is exposed"
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

impl WorkflowPublicationRuntimeRecovery {
    fn back_off(&mut self, now_ms: u64) {
        self.failures = self.failures.saturating_add(1);
        let exponent = self.failures.saturating_sub(1).min(6);
        let delay = PUBLICATION_RUNTIME_RECOVERY_BASE_DELAY_MS
            .saturating_mul(1_u64 << exponent)
            .min(PUBLICATION_RUNTIME_RECOVERY_MAX_DELAY_MS);
        self.next_attempt_at_ms = now_ms.saturating_add(delay);
    }
}

impl WorkflowPublicationRuntimeProcessStore {
    async fn claim_launch(&self, key: &str) -> bool {
        self.launching.lock().await.insert(key.to_string())
    }

    async fn launching(&self, key: &str) -> bool {
        self.launching.lock().await.contains(key)
    }

    async fn release_launch(&self, key: &str) {
        self.launching.lock().await.remove(key);
    }

    async fn recovery_due(&self, key: &str, now_ms: u64, package_digest: &str) -> bool {
        self.recoveries
            .lock()
            .await
            .get(key)
            .is_none_or(|recovery| match &recovery.parked_for_digest {
                Some(parked) => parked != package_digest,
                None => recovery.next_attempt_at_ms <= now_ms,
            })
    }

    async fn park_recovery(&self, key: &str, package_digest: &str) {
        let mut guard = self.recoveries.lock().await;
        let recovery = guard.entry(key.to_string()).or_default();
        recovery.parked_for_digest = Some(package_digest.to_string());
    }

    /// A running runtime clears its backoff, unless recovery launched it
    /// less than `PUBLICATION_RUNTIME_RECOVERY_STABLE_MS` ago.
    async fn record_recovery_running(&self, key: &str, now_ms: u64) {
        let mut guard = self.recoveries.lock().await;
        let settling = guard
            .get(key)
            .and_then(|recovery| recovery.launched_at_ms)
            .is_some_and(|launched| {
                now_ms < launched.saturating_add(PUBLICATION_RUNTIME_RECOVERY_STABLE_MS)
            });
        if !settling {
            guard.remove(key);
        }
    }

    /// A recovery launch counts as an attempt: if the runtime exits soon
    /// after, the next launch waits out a growing delay.
    async fn record_recovery_launch(&self, key: &str, now_ms: u64) {
        let mut guard = self.recoveries.lock().await;
        let recovery = guard.entry(key.to_string()).or_default();
        recovery.back_off(now_ms);
        recovery.launched_at_ms = Some(now_ms);
    }

    async fn record_recovery_failure(&self, key: &str, now_ms: u64) {
        let mut guard = self.recoveries.lock().await;
        let recovery = guard.entry(key.to_string()).or_default();
        recovery.back_off(now_ms);
        recovery.parked_for_digest = None;
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

    async fn keys(&self) -> Vec<String> {
        self.inner.lock().await.keys().cloned().collect()
    }
}

#[cfg(test)]
impl KernelRuntimeState {
    /// Registers a gateway that has just launched (a long-running child) as
    /// the publication's runtime; returns the outcome and the child's pid.
    pub(crate) async fn fixture_register_launched_runtime(
        &self,
        session_id: &str,
        publication_id: &str,
    ) -> (Result<(), DaemonError>, u32) {
        let child = Command::new("sleep")
            .arg("30")
            .kill_on_drop(true)
            .spawn()
            .expect("launched gateway");
        let process_id = child.id().expect("pid");
        let outcome = register_launched_runtime(
            self,
            session_id,
            publication_id,
            publication_runtime_process_key(session_id, publication_id),
            WorkflowPublicationRuntimeProcess {
                child,
                process_id: Some(process_id),
                host: DEFAULT_PUBLICATION_RUNTIME_HOST.to_string(),
                port: 0,
                local_url: None,
            },
        )
        .await;
        (outcome, process_id)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        exit_gateway_with_kernel, is_package_digest_mismatch, launched_publication_runtime_message,
        launched_publication_runtime_status, publication_local_url, publication_runtime_error,
        publication_runtime_launch_context, publication_runtime_metadata_preserving_binding,
        publication_runtime_port, publication_runtime_recovery_binding,
        stopped_publication_runtime_metadata, validate_bound_publication_package_digest,
        validate_bound_release, validate_publication_runtime_bind_address,
        validated_deployment_binding, write_publication_caller_claims_config, BoundReleaseDigests,
        PublicationRuntimeLaunchContext, WorkflowPublicationRuntimeProcessStore,
        DEFAULT_PUBLICATION_RUNTIME_PORT,
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
    fn launched_runtime_without_ingress_is_running_without_registration() {
        assert_eq!(launched_publication_runtime_status(true), "running");
        assert!(launched_publication_runtime_message(true).contains("no ingress endpoint"));
    }

    #[test]
    fn schedule_only_and_app_event_triggers_have_no_ingress() {
        let publication = |kind: &str| -> crate::session::WorkflowPublicationDefinition {
            serde_json::from_value(serde_json::json!({
                "id": "publication-1", "session_id": "session-1", "workflow_id": "workflow-1",
                "endpoint_id": "endpoint-1", "kind": kind, "enabled": true,
                "methods": [], "schedules": [], "watchdogs": [], "recent_runs": [],
                "runtime_logs": [], "created_by_user_id": "user-1", "created_at_ms": 1,
                "updated_at_ms": 1,
            }))
            .expect("publication")
        };
        assert!(super::publication_has_ingress(&publication("ingress")));
        assert!(!super::publication_has_ingress(&publication(
            "schedule_only"
        )));
        assert!(!super::publication_has_ingress(&publication("event_based")));
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
    fn bound_runtime_restart_requires_the_deployed_package_digest() {
        assert!(validate_bound_publication_package_digest("sha256:exact", "sha256:exact").is_ok());
        let error = validate_bound_publication_package_digest("sha256:exact", "sha256:changed")
            .expect_err("changed source must require a fresh deployment binding");
        assert!(error.contains("no longer matches the bound deployment"));
        assert!(error.contains("rebind the deployment before restarting"));
    }

    #[test]
    fn a_release_with_recorded_inputs_verifies_across_kernel_template_changes() {
        let mut publication: crate::session::WorkflowPublicationDefinition =
            serde_json::from_value(serde_json::json!({
                "id": "publication-1", "session_id": "session-1", "workflow_id": "workflow-1",
                "endpoint_id": "endpoint-1", "kind": "event_based", "enabled": true,
                "methods": [], "schedules": [], "watchdogs": [], "recent_runs": [],
                "runtime_logs": [], "created_by_user_id": "user-1", "created_at_ms": 1,
                "updated_at_ms": 1,
            }))
            .expect("publication");
        let digests = |package: &str, inputs: &str| BoundReleaseDigests {
            package: package.to_string(),
            inputs: inputs.to_string(),
        };
        // Before protocol 378 a release verifies by its whole package digest.
        assert!(validate_bound_release(
            &publication,
            "sha256:release",
            &digests("sha256:upgraded", "sha256:inputs")
        )
        .is_err());
        publication.record_release_inputs("sha256:release", "sha256:inputs");
        // A kernel upgrade changes the templates, not the workflow's inputs.
        assert_eq!(
            validate_bound_release(
                &publication,
                "sha256:release",
                &digests("sha256:upgraded", "sha256:inputs")
            ),
            Ok(())
        );
        let error = validate_bound_release(
            &publication,
            "sha256:release",
            &digests("sha256:release", "sha256:edited"),
        )
        .expect_err("changed workflow inputs must require a fresh deployment binding");
        assert!(error.contains("no longer matches the bound deployment"));
        assert!(error.contains("sha256:edited"));
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
    async fn an_ingressless_gateway_starts_once_it_consumed_its_caller_claims_config() {
        let config = std::env::temp_dir().join(format!(
            "chariox-caller-claims-{}-{}.json",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        // Removed however the test ends.
        struct Cleanup<'a>(&'a std::path::Path);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                let _ = fs::remove_file(self.0);
            }
        }
        let _cleanup = Cleanup(&config);
        fs::write(&config, "{}").expect("config");
        // The gateway consumes the file after it starts; the kernel waits for that.
        let mut child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(format!("sleep 0.3; rm {}; sleep 5", config.display()))
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("gateway");
        super::wait_for_publication_runtime_start(&mut child, "127.0.0.1", 0, true, Some(&config))
            .await
            .expect("started once the config is consumed");
        assert!(!config.exists());
        let _ = child.kill().await;

        // A gateway that exits before consuming it failed to start.
        fs::write(&config, "{}").expect("config");
        let mut child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg("echo no deployment config >&2; exit 1")
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("gateway");
        let error = super::wait_for_publication_runtime_start(
            &mut child,
            "127.0.0.1",
            0,
            true,
            Some(&config),
        )
        .await
        .expect_err("exited before starting");
        assert!(error.contains("exited before becoming ready"), "{error}");
        assert!(error.contains("no deployment config"), "{error}");
    }

    #[tokio::test]
    async fn gateway_exits_when_its_kernel_releases_the_parent_pipe() {
        // Stands in for the gateway: honours the parent-pipe contract by
        // exiting when its stdin closes, and otherwise runs on.
        let mut command = tokio::process::Command::new("sh");
        command
            .arg("-c")
            .arg("[ \"$CHARIOX_PUBLICATION_EXIT_ON_STDIN_CLOSE\" = 1 ] || exit 3; cat >/dev/null");
        exit_gateway_with_kernel(&mut command);
        let mut child = command.spawn().expect("spawn gateway stand-in");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(child.try_wait().expect("inspect gateway").is_none());
        let pid = child.id().expect("gateway pid") as libc::pid_t;
        // The kernel dying closes the pipe exactly as dropping its end does.
        drop(child.stdin.take());
        let status = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
            .await
            .unwrap_or_else(|_| {
                unsafe { libc::kill(pid, libc::SIGKILL) };
                panic!("gateway outlived its kernel's pipe");
            })
            .expect("wait for gateway");
        assert!(status.success());
    }

    #[tokio::test]
    async fn deployment_runtime_recovery_uses_bounded_exponential_backoff() {
        let store = WorkflowPublicationRuntimeProcessStore::default();
        assert!(store.recovery_due("publication-1", 100, "sha256:a").await);
        store.record_recovery_failure("publication-1", 100).await;
        assert!(!store.recovery_due("publication-1", 1_099, "sha256:a").await);
        assert!(store.recovery_due("publication-1", 1_100, "sha256:a").await);
        store.record_recovery_failure("publication-1", 1_100).await;
        assert!(!store.recovery_due("publication-1", 3_099, "sha256:a").await);
        assert!(store.recovery_due("publication-1", 3_100, "sha256:a").await);
        store.record_recovery_running("publication-1", 3_100).await;
        assert!(store.recovery_due("publication-1", 3_100, "sha256:a").await);
    }

    #[tokio::test]
    async fn digest_mismatch_recovery_waits_for_a_rebind() {
        let store = WorkflowPublicationRuntimeProcessStore::default();
        store.park_recovery("publication-1", "sha256:a").await;
        assert!(
            !store
                .recovery_due("publication-1", u64::MAX, "sha256:a")
                .await
        );
        assert!(store.recovery_due("publication-1", 0, "sha256:b").await);
        store.record_recovery_failure("publication-1", 0).await;
        assert!(!store.recovery_due("publication-1", 0, "sha256:a").await);
        assert!(
            store
                .recovery_due("publication-1", 60_000, "sha256:a")
                .await
        );

        let mismatch = validate_bound_publication_package_digest("sha256:a", "sha256:b")
            .map_err(|message| {
                publication_runtime_error("start workflow publication runtime", message)
            })
            .unwrap_err();
        assert!(is_package_digest_mismatch(&mismatch));
        assert!(!is_package_digest_mismatch(&publication_runtime_error(
            "start workflow publication runtime",
            "publication gateway exited",
        )));
    }

    #[tokio::test]
    async fn a_recovered_runtime_that_exits_soon_waits_before_relaunching() {
        let store = WorkflowPublicationRuntimeProcessStore::default();
        let stable = super::PUBLICATION_RUNTIME_RECOVERY_STABLE_MS;
        store.record_recovery_launch("publication-1", 100).await;
        // Seen running on a tick, then gone: no immediate relaunch.
        store.record_recovery_running("publication-1", 600).await;
        assert!(!store.recovery_due("publication-1", 1_099, "sha256:a").await);
        assert!(store.recovery_due("publication-1", 1_100, "sha256:a").await);
        // Each launch that does not stay up waits longer.
        store.record_recovery_launch("publication-1", 1_100).await;
        assert!(!store.recovery_due("publication-1", 3_099, "sha256:a").await);
        // Up for the stable period: the backoff is cleared.
        store
            .record_recovery_running("publication-1", 1_100 + stable)
            .await;
        store
            .record_recovery_launch("publication-1", 1_100 + stable)
            .await;
        assert!(
            store
                .recovery_due("publication-1", 2_100 + stable, "sha256:a")
                .await
        );
    }
}
