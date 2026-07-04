//! Kernel-owned lifecycle control for local workflow publication runtimes.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use base64::Engine;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::error::DaemonError;
use crate::local::{
    ControlWorkflowPublicationRuntimeRequest, LocalDaemonResponse, WorkflowPublicationPackageFile,
    WorkflowPublicationRuntimeAction,
};
use crate::session::WorkflowPublicationDefinition;

use super::KernelRuntimeState;

const DEFAULT_PUBLICATION_RUNTIME_HOST: &str = "127.0.0.1";
const DEFAULT_PUBLICATION_RUNTIME_PORT: u16 = 3000;

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
        WorkflowPublicationRuntimeAction::Stop => {
            stop_publication_runtime(runtime_state, &process_key).await?;
            let publication = mark_publication_runtime_status(
                runtime_state,
                &request.session_id,
                &publication_id,
                "stopped",
                None,
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
    let port = request.port.unwrap_or(DEFAULT_PUBLICATION_RUNTIME_PORT);
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
            existing.local_url.clone(),
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
        return Ok(LocalDaemonResponse::WorkflowPublicationRuntimeControlled {
            publication: refreshed,
            action: WorkflowPublicationRuntimeAction::Start,
            status: "running".to_string(),
            local_url: existing.local_url,
            open_url,
            process_id: existing.process_id,
            message: Some("publication runtime is already running".to_string()),
        });
    }

    let package = runtime_state.owned.workflow_export_publication_package(
        crate::local::ExportWorkflowPublicationPackageRequest {
            session_id: request.session_id.clone(),
            publication_ref: publication.id().to_string(),
            kernel_url: request.kernel_url.clone(),
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
    let local_url = if is_schedule_only_publication(&publication) {
        None
    } else {
        Some(publication_local_url(&host, port, publication.route()))
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
        .stderr(Stdio::null());
    if let Some(kernel_url) = request.kernel_url.as_deref().and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    }) {
        command.arg("--kernel-url").arg(kernel_url);
    }
    let mut child = command
        .spawn()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "start workflow publication runtime",
            message: format!("failed to launch arroba publication gateway: {error}"),
        })?;
    let process_id = child.id();
    if let Some(status) = child
        .try_wait()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "start workflow publication runtime",
            message: format!("failed to inspect launched publication gateway: {error}"),
        })?
    {
        return Err(DaemonError::LocalTransport {
            operation: "start workflow publication runtime",
            message: format!("publication gateway exited immediately with status {status}"),
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
    let publication = mark_publication_runtime_status(
        runtime_state,
        &request.session_id,
        publication.id(),
        "starting",
        local_url.clone(),
        Some(serde_json::json!({
            "kind": "local_runtime",
            "status": "starting",
            "host": host,
            "port": port,
            "local_url": local_url,
            "process_id": process_id,
            "package_root": package_root,
        })),
    )?;
    Ok(LocalDaemonResponse::WorkflowPublicationRuntimeControlled {
        publication,
        action: request.action,
        status: "starting".to_string(),
        local_url: local_url.clone(),
        open_url: local_url,
        process_id,
        message: Some("publication runtime starting; endpoint registration will publish a relay display URL when available".to_string()),
    })
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
        .map_err(|error| DaemonError::LocalTransport {
            operation: "stop workflow publication runtime",
            message: format!("failed to inspect publication gateway: {error}"),
        })?
        .is_none()
    {
        process
            .child
            .kill()
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "stop workflow publication runtime",
                message: format!("failed to stop publication gateway: {error}"),
            })?;
    }
    Ok(())
}

fn mark_publication_runtime_status(
    runtime_state: &KernelRuntimeState,
    session_id: &str,
    publication_ref: &str,
    status: &str,
    open_url: Option<String>,
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

fn publication_local_url(host: &str, port: u16, route: Option<&str>) -> String {
    let base = format!("http://{}:{}", host, port);
    match route.map(str::trim).filter(|value| !value.is_empty()) {
        Some(route) => {
            let normalized = if route.starts_with('/') {
                route.to_string()
            } else {
                format!("/{route}")
            };
            format!("{}{}", base, normalized.trim_end_matches('*'))
        }
        None => base,
    }
}

fn is_schedule_only_publication(publication: &WorkflowPublicationDefinition) -> bool {
    publication
        .transport()
        .and_then(|transport| transport.get("kind"))
        .and_then(serde_json::Value::as_str)
        == Some("schedule_only")
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
