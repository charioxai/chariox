use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::error::DaemonError;
use crate::slice_provider_auth::SliceProviderAuthSummary;

use super::model::{
    CreateSliceInput, SliceBackendKind, SliceDisplayEndpoint, SliceDisplayEndpointAccess,
    SliceDisplayEndpointKind, SliceDisplayMode, SliceRecord, SliceRelayEndpoint, SliceStatus,
};
use super::ports::{self, LocalDockerSlicePorts};

#[derive(Debug, Clone, Default)]
pub struct SliceStore {
    inner: Arc<Mutex<SliceStoreState>>,
}

#[derive(Debug)]
pub struct SliceOperationGuard {
    store: SliceStore,
    slice_id: String,
    operation: String,
}

impl Drop for SliceOperationGuard {
    fn drop(&mut self) {
        let mut state = self.store.inner.lock().expect("slice store poisoned");
        if state
            .active_operations
            .get(&self.slice_id)
            .is_some_and(|operation| operation == &self.operation)
        {
            state.active_operations.remove(&self.slice_id);
        }
    }
}

#[derive(Debug, Default)]
struct SliceStoreState {
    next_slice_number: u64,
    records: BTreeMap<String, SliceRecord>,
    active_operations: BTreeMap<String, String>,
}

impl SliceStore {
    pub fn create(
        &self,
        owner_kernel_id: &str,
        owner_machine_id: &str,
        input: CreateSliceInput,
    ) -> Result<SliceRecord, DaemonError> {
        validate_slice_name(&input.name)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        if state
            .records
            .values()
            .any(|record| record.name == input.name || record.id == input.name)
        {
            return Err(DaemonError::LocalTransport {
                operation: "slice.create",
                message: format!("slice `{}` already exists", input.name),
            });
        }
        state.next_slice_number = state.next_slice_number.saturating_add(1);
        let id = format!("slice-{}", state.next_slice_number);
        let worker_kernel_ref = input
            .worker_kernel_ref
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("slice:{}", input.name));
        let local_docker_ports = if input.backend == SliceBackendKind::LocalDocker {
            Some(ports::allocate_local_docker_ports_for_slice(
                &state.records,
            )?)
        } else {
            None
        };
        let display_url = if input.display_mode == SliceDisplayMode::Headed {
            input.display_url.or_else(|| {
                let ports = local_docker_ports
                    .map(LocalDockerSlicePorts::from_assignment)
                    .unwrap_or_else(|| LocalDockerSlicePorts::for_slice_id(&id));
                Some(format!(
                    "http://127.0.0.1:{}/vnc.html?autoconnect=true",
                    ports.novnc
                ))
            })
        } else {
            None
        };
        let display_endpoint = display_url.map(|url| SliceDisplayEndpoint {
            slice_id: id.clone(),
            kind: SliceDisplayEndpointKind::Novnc,
            url,
            access: SliceDisplayEndpointAccess::Local,
            expires_at_ms: None,
            capabilities: vec![
                "view".to_string(),
                "keyboard".to_string(),
                "mouse".to_string(),
            ],
        });
        let record = SliceRecord {
            id: id.clone(),
            name: input.name,
            owner_kernel_id: owner_kernel_id.to_string(),
            owner_machine_id: owner_machine_id.to_string(),
            session_id: None,
            session_ids: Vec::new(),
            agent_ids: Vec::new(),
            backend: input.backend,
            os: input.os,
            display_mode: input.display_mode,
            status: SliceStatus::Stopped,
            workspace_id: input.workspace_id,
            worktree_id: input.worktree_id,
            workspace_mount: input.workspace_mount,
            worker_kernel_ref,
            worker_kernel_id: None,
            worker_machine_id: None,
            relay_endpoint: None,
            local_docker_ports,
            providers: Vec::new(),
            provider_auth: input.provider_auth,
            display_endpoint,
            created_at_ms: input.now_ms,
            updated_at_ms: input.now_ms,
        };
        state.records.insert(id, record.clone());
        Ok(record)
    }

    pub fn list(&self) -> Vec<SliceRecord> {
        let state = self.inner.lock().expect("slice store poisoned");
        state.records.values().cloned().collect()
    }

    pub fn restore_records(&self, records: Vec<SliceRecord>) {
        let mut state = self.inner.lock().expect("slice store poisoned");
        state.records.clear();
        state.next_slice_number = 0;
        for record in records {
            if let Some(number) = record
                .id
                .strip_prefix("slice-")
                .and_then(|value| value.parse::<u64>().ok())
            {
                state.next_slice_number = state.next_slice_number.max(number);
            }
            state.records.insert(record.id.clone(), record);
        }
    }

    pub fn reconcile_after_kernel_restart(&self, now_ms: u64) -> Vec<SliceRecord> {
        let mut state = self.inner.lock().expect("slice store poisoned");
        let mut changed = Vec::new();
        for record in state.records.values_mut() {
            let was_runtime_status = matches!(
                record.status,
                SliceStatus::Starting | SliceStatus::Running | SliceStatus::Unhealthy
            );
            let mut record_changed = false;
            if matches!(record.status, SliceStatus::Starting | SliceStatus::Running) {
                record.status = SliceStatus::Unhealthy;
                record_changed = true;
            }
            if was_runtime_status {
                if record.worker_kernel_id.take().is_some() {
                    record_changed = true;
                }
                if record.worker_machine_id.take().is_some() {
                    record_changed = true;
                }
                if record.relay_endpoint.take().is_some() {
                    record_changed = true;
                }
                if !record.providers.is_empty() {
                    record.providers.clear();
                    record_changed = true;
                }
            }
            if record_changed {
                record.updated_at_ms = now_ms;
                changed.push(record.clone());
            }
        }
        changed
    }

    pub fn try_begin_operation(
        &self,
        slice_ref: &str,
        operation: &'static str,
    ) -> Result<SliceOperationGuard, DaemonError> {
        let slice_ref = slice_ref.trim();
        if slice_ref.is_empty() {
            return Err(DaemonError::LocalTransport {
                operation: "slice.operation",
                message: "slice reference must not be empty".to_string(),
            });
        }
        let mut state = self.inner.lock().expect("slice store poisoned");
        let mut matches = state
            .records
            .values()
            .filter(|record| record.id == slice_ref || record.name == slice_ref)
            .map(|record| record.id.clone())
            .collect::<Vec<_>>();
        let slice_id = match matches.len() {
            0 => {
                return Err(DaemonError::LocalTransport {
                    operation: "slice.operation",
                    message: format!("unknown slice `{slice_ref}`"),
                });
            }
            1 => matches.remove(0),
            _ => {
                return Err(DaemonError::LocalTransport {
                    operation: "slice.operation",
                    message: format!("slice reference `{slice_ref}` is ambiguous"),
                });
            }
        };
        if let Some(existing) = state.active_operations.get(&slice_id) {
            let record_name = state
                .records
                .get(&slice_id)
                .map(|record| record.name.as_str())
                .unwrap_or(slice_ref);
            return Err(DaemonError::LocalTransport {
                operation: "slice.operation",
                message: format!(
                    "slice `{record_name}` already has an active `{existing}` operation"
                ),
            });
        }
        state
            .active_operations
            .insert(slice_id.clone(), operation.to_string());
        Ok(SliceOperationGuard {
            store: self.clone(),
            slice_id,
            operation: operation.to_string(),
        })
    }

    pub fn resolve(&self, slice_ref: &str) -> Result<SliceRecord, DaemonError> {
        let slice_ref = slice_ref.trim();
        if slice_ref.is_empty() {
            return Err(DaemonError::LocalTransport {
                operation: "slice.resolve",
                message: "slice reference must not be empty".to_string(),
            });
        }
        let state = self.inner.lock().expect("slice store poisoned");
        let mut matches = state
            .records
            .values()
            .filter(|record| record.id == slice_ref || record.name == slice_ref)
            .cloned()
            .collect::<Vec<_>>();
        match matches.len() {
            0 => Err(DaemonError::LocalTransport {
                operation: "slice.resolve",
                message: format!("unknown slice `{slice_ref}`"),
            }),
            1 => Ok(matches.remove(0)),
            _ => Err(DaemonError::LocalTransport {
                operation: "slice.resolve",
                message: format!("slice reference `{slice_ref}` is ambiguous"),
            }),
        }
    }

    pub fn set_status(
        &self,
        slice_ref: &str,
        status: SliceStatus,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.status",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        record.status = status;
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn set_relay_endpoint(
        &self,
        slice_ref: &str,
        endpoint: Option<SliceRelayEndpoint>,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.relay_endpoint",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        record.relay_endpoint = endpoint;
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn set_worker_presence(
        &self,
        slice_ref: &str,
        worker_kernel_id: Option<String>,
        worker_machine_id: Option<String>,
        providers: Vec<String>,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.worker_presence",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        record.worker_kernel_id = worker_kernel_id;
        record.worker_machine_id = worker_machine_id;
        record.providers = providers;
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn set_provider_auth(
        &self,
        slice_ref: &str,
        provider_auth: Vec<SliceProviderAuthSummary>,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.provider_auth",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        record.provider_auth = provider_auth;
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn set_provider_auth_alias(
        &self,
        slice_ref: &str,
        provider: &str,
        alias: Option<&str>,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let provider = provider.trim();
        if provider.is_empty() {
            return Err(DaemonError::LocalTransport {
                operation: "slice.provider_auth_alias",
                message: "provider must not be empty".to_string(),
            });
        }
        let alias = alias
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let mut state = self.inner.lock().expect("slice store poisoned");
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.provider_auth_alias",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        let Some(auth) = record
            .provider_auth
            .iter_mut()
            .find(|auth| auth.provider == provider)
        else {
            return Err(DaemonError::LocalTransport {
                operation: "slice.provider_auth_alias",
                message: format!("slice `{}` has no `{provider}` auth summary", record.name),
            });
        };
        auth.alias = alias;
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn delete(&self, slice_ref: &str) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        if !resolved.agent_ids.is_empty() {
            return Err(DaemonError::LocalTransport {
                operation: "slice.delete",
                message: format!(
                    "slice `{}` still has {} active agent(s)",
                    resolved.name,
                    resolved.agent_ids.len()
                ),
            });
        }
        state
            .records
            .remove(&resolved.id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "slice.delete",
                message: format!("unknown slice `{slice_ref}`"),
            })
    }

    pub fn attach_session(
        &self,
        slice_ref: &str,
        session_id: &str,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.attach_session",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        if !record.session_ids.iter().any(|value| value == session_id) {
            record.session_ids.push(session_id.to_string());
        }
        record.session_id = Some(session_id.to_string());
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn detach_session(
        &self,
        slice_ref: &str,
        session_id: &str,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.detach_session",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        record.session_ids.retain(|value| value != session_id);
        if record.session_id.as_deref() == Some(session_id) {
            record.session_id = record.session_ids.last().cloned();
        }
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn attach_agent(
        &self,
        slice_ref: &str,
        session_id: &str,
        agent_id: &str,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.attach_agent",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        if !record.session_ids.iter().any(|value| value == session_id) {
            record.session_ids.push(session_id.to_string());
        }
        record.session_id = Some(session_id.to_string());
        if !record.agent_ids.iter().any(|value| value == agent_id) {
            record.agent_ids.push(agent_id.to_string());
        }
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn detach_agent(
        &self,
        slice_ref: &str,
        agent_id: &str,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.detach_agent",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        record.agent_ids.retain(|value| value != agent_id);
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn ensure_worktree_scope(
        &self,
        slice_ref: &str,
        workspace_id: Option<&str>,
        worktree_id: Option<&str>,
    ) -> Result<SliceRecord, DaemonError> {
        let record = self.resolve(slice_ref)?;
        if let (Some(expected), Some(actual)) = (workspace_id, record.workspace_id.as_deref()) {
            if expected != actual {
                return Err(DaemonError::LocalTransport {
                    operation: "slice.scope",
                    message: format!(
                        "slice `{}` belongs to workspace `{actual}`, not `{expected}`",
                        record.name
                    ),
                });
            }
        }
        if let (Some(expected), Some(actual)) = (worktree_id, record.worktree_id.as_deref()) {
            if expected != actual {
                return Err(DaemonError::LocalTransport {
                    operation: "slice.scope",
                    message: format!(
                        "slice `{}` belongs to worktree `{actual}`, not `{expected}`",
                        record.name
                    ),
                });
            }
        }
        Ok(record)
    }

    pub fn list_by_session(&self, session_id: &str) -> Vec<SliceRecord> {
        let state = self.inner.lock().expect("slice store poisoned");
        state
            .records
            .values()
            .filter(|record| {
                record.session_id.as_deref() == Some(session_id)
                    || record.session_ids.iter().any(|value| value == session_id)
            })
            .cloned()
            .collect()
    }

    pub fn resolve_worker_kernel_ref(&self, slice_ref: &str) -> Result<String, DaemonError> {
        let record = self.resolve(slice_ref)?;
        Ok(record
            .worker_kernel_id
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(record.worker_kernel_ref))
    }

    pub fn resolve_by_worker_kernel_ref(&self, kernel_ref: &str) -> Option<SliceRecord> {
        let kernel_ref = kernel_ref.trim();
        if kernel_ref.is_empty() {
            return None;
        }
        let state = self.inner.lock().expect("slice store poisoned");
        state
            .records
            .values()
            .find(|record| {
                record.worker_kernel_ref == kernel_ref
                    || record.worker_kernel_id.as_deref() == Some(kernel_ref)
                    || record.worker_machine_id.as_deref() == Some(kernel_ref)
            })
            .cloned()
    }

    pub fn display_endpoint(&self, slice_ref: &str) -> Result<SliceDisplayEndpoint, DaemonError> {
        let record = self.resolve(slice_ref)?;
        record
            .display_endpoint
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "slice.display_endpoint",
                message: format!("slice `{}` has no display endpoint", record.name),
            })
    }
}

fn validate_slice_name(name: &str) -> Result<(), DaemonError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "slice.validate",
            message: "slice name must not be empty".to_string(),
        });
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        return Err(DaemonError::LocalTransport {
            operation: "slice.validate",
            message: "slice name may only contain ASCII letters, numbers, '-', '_' or '.'"
                .to_string(),
        });
    }
    Ok(())
}
