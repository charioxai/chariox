//! Kernel-owned lifecycle control for local workflow publication runtimes.

use std::collections::BTreeMap;
use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use base64::Engine;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration, Instant};

use crate::error::DaemonError;
use crate::local::{
    ControlWorkflowPublicationRuntimeRequest, LocalDaemonResponse, WorkflowPublicationPackageFile,
    WorkflowPublicationRuntimeAction,
};
use crate::session::WorkflowPublicationDefinition;

use super::KernelRuntimeState;

const DEFAULT_PUBLICATION_RUNTIME_HOST: &str = "127.0.0.1";
const DEFAULT_PUBLICATION_RUNTIME_PORT: u16 = 3000;
const PUBLICATION_RUNTIME_START_TIMEOUT: Duration = Duration::from_secs(10);
const PUBLICATION_RUNTIME_START_POLL: Duration = Duration::from_millis(50);

#[derive(Clone, Default)]
pub(crate) struct WorkflowPublicationRuntimeProcessStore {
    inner: Arc<Mutex<BTreeMap<String, WorkflowPublicationRuntimeProcess>>>,
}

struct WorkflowPublicationRuntimeProcess {
    child: Child,
    process_id: Option<u32>,
    host: String,
    port: u16,
    local_url: Option<String>,
    package_root: PathBuf,
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
                Some(serde_json::json!({
                    "kind": "local_runtime",
                    "status": "stopped",
                })),
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
            start_publication_runtime(runtime_state, request, publication, process_key).await
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
    let publication = if let Some(process) = running.as_ref() {
        mark_publication_runtime_status(
            runtime_state,
            publication.session_id(),
            publication.id(),
            "running",
            Some(local_url.clone()),
            Some(serde_json::json!({
                "kind": "local_runtime",
                "status": "running",
                "host": process.host,
                "port": process.port,
                "local_url": process.local_url,
                "process_id": process.process_id,
                "package_root": process.package_root,
            })),
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
            Some(serde_json::json!({
                "kind": "local_runtime",
                "status": "stopped",
            })),
        )?
    };
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

async fn start_publication_runtime(
    runtime_state: &KernelRuntimeState,
    request: ControlWorkflowPublicationRuntimeRequest,
    publication: WorkflowPublicationDefinition,
    process_key: String,
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
            Some(serde_json::json!({
                "kind": "local_runtime",
                "status": "running",
                "host": existing.host,
                "port": existing.port,
                "local_url": existing.local_url,
                "process_id": existing.process_id,
                "package_root": existing.package_root,
            })),
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
    let package = runtime_state.owned.workflow_export_publication_package(
        crate::local::ExportWorkflowPublicationPackageRequest {
            session_id: request.session_id.clone(),
            publication_ref: publication.id().to_string(),
            kernel_url: Some(kernel_url.clone()),
            agent_app: None,
            agent_app_assets_dir: None,
        },
    )?;
    let LocalDaemonResponse::WorkflowPublicationPackageExported {
        package_digest,
        package_files,
        ..
    } = package
    else {
        return Err(DaemonError::LocalTransport {
            operation: "start workflow publication runtime",
            message: "publication package export returned an unexpected response".to_string(),
        });
    };
    let package_root = materialize_publication_package(
        &request.session_id,
        publication.id(),
        &package_digest,
        &package_files,
    )?;
    let local_url = if is_schedule_only {
        None
    } else {
        Some(publication_local_url(&host, port))
    };
    let mut command = Command::new(resolve_arroba_cli_bin()?);
    command
        .arg("serve")
        .arg(&package_root)
        .arg(port.to_string())
        .arg("--host")
        .arg(&host)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command.arg("--kernel-url").arg(&kernel_url);
    let mut child = command.spawn().map_err(|error| {
        let message = format!("failed to launch arroba publication gateway: {error}");
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
                package_root: package_root.clone(),
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
        Some(serde_json::json!({
            "kind": "local_runtime",
            "status": runtime_status,
            "host": host,
            "port": port,
            "local_url": local_url,
            "kernel_url": kernel_url,
            "process_id": process_id,
            "package_root": package_root,
        })),
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

fn materialize_publication_package(
    session_id: &str,
    publication_id: &str,
    package_digest: &str,
    package_files: &[WorkflowPublicationPackageFile],
) -> Result<PathBuf, DaemonError> {
    let root = publication_runtime_root()
        .join(safe_path_segment(session_id))
        .join(format!(
            "{}-{}",
            safe_path_segment(publication_id),
            safe_path_segment(package_digest)
        ));
    if root.exists() {
        fs::remove_dir_all(&root).map_err(|error| DaemonError::LocalTransport {
            operation: "materialize workflow publication runtime package",
            message: format!(
                "failed to replace package directory `{}`: {error}",
                root.display()
            ),
        })?;
    }
    fs::create_dir_all(&root).map_err(|error| DaemonError::LocalTransport {
        operation: "materialize workflow publication runtime package",
        message: format!(
            "failed to create package directory `{}`: {error}",
            root.display()
        ),
    })?;
    for file in package_files {
        let relative = safe_relative_package_path(&file.path)?;
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| DaemonError::LocalTransport {
                operation: "materialize workflow publication runtime package",
                message: format!(
                    "failed to create package directory `{}`: {error}",
                    parent.display()
                ),
            })?;
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&file.content_base64)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "materialize workflow publication runtime package",
                message: format!("failed to decode package file `{}`: {error}", file.path),
            })?;
        fs::write(&path, bytes).map_err(|error| DaemonError::LocalTransport {
            operation: "materialize workflow publication runtime package",
            message: format!("failed to write package file `{}`: {error}", path.display()),
        })?;
        #[cfg(unix)]
        if file.executable {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&path)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "materialize workflow publication runtime package",
                    message: format!(
                        "failed to inspect package file `{}`: {error}",
                        path.display()
                    ),
                })?
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&path, permissions).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "materialize workflow publication runtime package",
                    message: format!(
                        "failed to mark package file executable `{}`: {error}",
                        path.display()
                    ),
                }
            })?;
        }
    }
    Ok(root)
}

fn safe_relative_package_path(value: &str) -> Result<PathBuf, DaemonError> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(DaemonError::LocalTransport {
            operation: "materialize workflow publication runtime package",
            message: format!("unsafe package file path `{value}`"),
        });
    }
    Ok(path.to_path_buf())
}

fn publication_runtime_root() -> PathBuf {
    std::env::var_os("ARROBA_PUBLICATION_RUNTIME_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("arroba-publication-runtimes"))
}

fn resolve_arroba_cli_bin() -> Result<PathBuf, DaemonError> {
    if let Some(value) = std::env::var_os("ARROBA_CLI_BIN") {
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
        "arroba-cli.exe"
    } else {
        "arroba-cli"
    });
    if candidate.exists() {
        return Ok(candidate);
    }
    Err(DaemonError::LocalTransport {
        operation: "start workflow publication runtime",
        message: format!(
            "arroba-cli was not found beside `{}`; set ARROBA_CLI_BIN to enable publication runtime lifecycle",
            current.display()
        ),
    })
}

fn publication_runtime_process_key(session_id: &str, publication_id: &str) -> String {
    format!("{session_id}:{publication_id}")
}

fn safe_path_segment(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
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
    package_root: PathBuf,
}

impl WorkflowPublicationRuntimeProcessStore {
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
            package_root: process.package_root.clone(),
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
        publication_local_url, publication_runtime_port, validate_publication_runtime_bind_address,
        DEFAULT_PUBLICATION_RUNTIME_PORT,
    };
    use std::net::TcpListener;

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
}
