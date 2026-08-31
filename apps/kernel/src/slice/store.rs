use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::error::DaemonError;
use crate::slice_provider_auth::SliceProviderAuthSummary;

use super::model::{
    CreateSliceInput, SliceBackendKind, SliceBackupRecord, SliceDisplayEndpoint,
    SliceOperationStatus, SliceRecord, SliceRelayEndpoint, SliceSavedStateRecord,
    SliceSavedStateStatus, SliceStatus,
};
use super::ports::{self, LocalDockerSlicePorts};

mod environment;
mod invariants;

use invariants::{
    reconcile_slice_status_after_kernel_restart, redact_slice_operation_error, validate_slice_name,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceHostRuntimeState {
    Running,
    Stopped,
    Missing,
    Unknown,
}

#[derive(Debug, Clone, Default)]
pub struct SliceStore {
    inner: Arc<Mutex<SliceStoreState>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceAgentAttachment {
    pub slice_ref: String,
    pub session_id: String,
    pub agent_id: String,
}

#[derive(Debug)]
pub struct SliceOperationGuard {
    store: SliceStore,
    slice_id: String,
    operation: String,
}

impl Drop for SliceOperationGuard {
    fn drop(&mut self) {
        let mut state = self
            .store
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
    saved_states: BTreeMap<String, SliceSavedStateRecord>,
    backups: BTreeMap<String, SliceBackupRecord>,
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
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let display_port = local_docker_ports
            .map(LocalDockerSlicePorts::from_assignment)
            .unwrap_or_else(|| LocalDockerSlicePorts::for_slice_id(&id))
            .novnc;
        let display_endpoint = super::display::display_endpoint_for_slice(
            &id,
            &input.display_mode,
            input.display_backend,
            display_port,
            input.display_url,
        );
        let from_saved_state = input.from_saved_state.clone();
        let record = SliceRecord {
            id: id.clone(),
            name: input.name,
            owner_kernel_id: owner_kernel_id.to_string(),
            owner_machine_id: owner_machine_id.to_string(),
            environment_session_id: None,
            session_id: None,
            session_ids: Vec::new(),
            agent_ids: Vec::new(),
            backend: input.backend,
            os: input.os,
            display_mode: input.display_mode,
            status: SliceStatus::Stopped,
            last_operation: Some("create".to_string()),
            last_operation_status: Some(SliceOperationStatus::Completed),
            last_error: None,
            last_operation_at_ms: Some(input.now_ms),
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
            saved_state_ref: from_saved_state.as_ref().map(|state| state.id.clone()),
            saved_state_status: from_saved_state
                .as_ref()
                .map(|_| SliceSavedStateStatus::Saved),
            saved_state_updated_at_ms: from_saved_state.as_ref().map(|state| state.updated_at_ms),
            display_endpoint,
            created_at_ms: input.now_ms,
            updated_at_ms: input.now_ms,
        };
        if let Some(saved_state) = from_saved_state {
            state
                .saved_states
                .insert(saved_state.id.clone(), saved_state);
        }
        state.records.insert(id, record.clone());
        Ok(record)
    }

    pub fn list(&self) -> Vec<SliceRecord> {
        let state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.records.values().cloned().collect()
    }

    pub fn restore_records(&self, records: Vec<SliceRecord>) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    pub fn saved_state(&self, state_ref: &str) -> Result<SliceSavedStateRecord, DaemonError> {
        let state_ref = state_ref.trim();
        if state_ref.is_empty() {
            return Err(DaemonError::LocalTransport {
                operation: "slice.state.get",
                message: "saved state reference must not be empty".to_string(),
            });
        }
        let state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .saved_states
            .values()
            .find(|record| record.id == state_ref || record.slice_name == state_ref)
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "slice.state.get",
                message: format!("unknown saved slice state `{state_ref}`"),
            })
    }

    pub fn active_saved_state_for_slice(
        &self,
        slice_ref: &str,
    ) -> Result<Option<SliceSavedStateRecord>, DaemonError> {
        let slice = self.resolve(slice_ref)?;
        let state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(slice
            .saved_state_ref
            .as_ref()
            .and_then(|state_ref| state.saved_states.get(state_ref))
            .cloned())
    }

    pub fn list_saved_states(&self) -> Vec<SliceSavedStateRecord> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .saved_states
            .values()
            .cloned()
            .collect()
    }

    pub fn list_backups(&self) -> Vec<SliceBackupRecord> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .backups
            .values()
            .cloned()
            .collect()
    }

    pub fn restore_saved_state_records(
        &self,
        states: Vec<SliceSavedStateRecord>,
        backups: Vec<SliceBackupRecord>,
    ) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.saved_states.clear();
        state.backups.clear();
        for saved in states {
            state.saved_states.insert(saved.id.clone(), saved);
        }
        for backup in backups {
            state.backups.insert(backup.id.clone(), backup);
        }
    }

    pub fn upsert_saved_state(
        &self,
        slice_ref: &str,
        saved_state: SliceSavedStateRecord,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .saved_states
            .insert(saved_state.id.clone(), saved_state.clone());
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.state.save",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        record.saved_state_ref = Some(saved_state.id);
        record.saved_state_status = Some(SliceSavedStateStatus::Saved);
        record.saved_state_updated_at_ms = Some(now_ms);
        record.last_operation = Some("state.save".to_string());
        record.last_operation_status = Some(SliceOperationStatus::Completed);
        record.last_error = None;
        record.last_operation_at_ms = Some(now_ms);
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn mark_saved_state_failed(
        &self,
        slice_ref: &str,
        error: &DaemonError,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.state.save",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        record.saved_state_status = Some(SliceSavedStateStatus::Failed);
        record.last_operation = Some("state.save".to_string());
        record.last_operation_status = Some(SliceOperationStatus::Failed);
        record.last_error = Some(redact_slice_operation_error(&error.to_string()));
        record.last_operation_at_ms = Some(now_ms);
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn reset_saved_state(
        &self,
        slice_ref: &str,
        now_ms: u64,
    ) -> Result<(SliceRecord, Option<SliceSavedStateRecord>), DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (updated_record, removed_state_ref) =
            {
                let record = state.records.get_mut(&resolved.id).ok_or_else(|| {
                    DaemonError::LocalTransport {
                        operation: "slice.state.reset",
                        message: format!("unknown slice `{slice_ref}`"),
                    }
                })?;
                let removed_state_ref = record.saved_state_ref.take();
                record.saved_state_status = None;
                record.saved_state_updated_at_ms = None;
                record.last_operation = Some("state.reset".to_string());
                record.last_operation_status = Some(SliceOperationStatus::Completed);
                record.last_error = None;
                record.last_operation_at_ms = Some(now_ms);
                record.updated_at_ms = now_ms;
                (record.clone(), removed_state_ref)
            };
        let removed = removed_state_ref.and_then(|state_ref| state.saved_states.remove(&state_ref));
        Ok((updated_record, removed))
    }

    pub fn upsert_backup(&self, backup: SliceBackupRecord) -> SliceBackupRecord {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.backups.insert(backup.id.clone(), backup.clone());
        backup
    }

    pub fn reconcile_after_kernel_restart(&self, now_ms: u64) -> Vec<SliceRecord> {
        self.reconcile_after_kernel_restart_with_host_state(now_ms, |_| {
            SliceHostRuntimeState::Unknown
        })
    }

    pub fn reconcile_after_kernel_restart_with_host_state(
        &self,
        now_ms: u64,
        inspect_host_runtime: impl Fn(&SliceRecord) -> SliceHostRuntimeState,
    ) -> Vec<SliceRecord> {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut changed = Vec::new();
        for record in state.records.values_mut() {
            let host_runtime = inspect_host_runtime(record);
            let was_runtime_status = matches!(
                record.status,
                SliceStatus::Starting
                    | SliceStatus::Stopping
                    | SliceStatus::Running
                    | SliceStatus::Unhealthy
            );
            let mut record_changed = false;
            let reconciled_status =
                reconcile_slice_status_after_kernel_restart(record.status.clone(), host_runtime);
            if record.status != reconciled_status {
                record.status = reconciled_status;
                record_changed = true;
            }
            // A running container survives a home-kernel restart. Its worker
            // identity, provider inventory, and relay endpoint remain valid;
            // clearing them strands the live slice until it is restarted.
            // Only discard runtime fields when the host no longer confirms
            // that the slice is running.
            let runtime_fields_are_stale = !matches!(host_runtime, SliceHostRuntimeState::Running)
                && (was_runtime_status || matches!(record.status, SliceStatus::Stopped));
            if runtime_fields_are_stale {
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
                record.last_operation = Some("restart_reconcile".to_string());
                record.last_operation_status = Some(SliceOperationStatus::Reconciled);
                record.last_error = match record.status {
                    SliceStatus::Unhealthy => Some(format!(
                        "slice `{}` needs restart or inspection after kernel restart",
                        record.name
                    )),
                    _ => None,
                };
                record.last_operation_at_ms = Some(now_ms);
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
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    pub fn set_operation_diagnostics(
        &self,
        slice_ref: &str,
        operation_name: &str,
        operation_status: SliceOperationStatus,
        error: Option<&str>,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.operation_diagnostics",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        record.last_operation = Some(operation_name.to_string());
        record.last_operation_status = Some(operation_status);
        record.last_error = error.map(redact_slice_operation_error);
        record.last_operation_at_ms = Some(now_ms);
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
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    pub fn delete(&self, slice_ref: &str) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    pub fn attach_agents(
        &self,
        attachments: Vec<SliceAgentAttachment>,
        now_ms: u64,
    ) -> Result<Vec<SliceRecord>, DaemonError> {
        if attachments.is_empty() {
            return Ok(Vec::new());
        }
        let resolved = attachments
            .iter()
            .map(|attachment| {
                self.resolve(&attachment.slice_ref)
                    .map(|slice| (slice.id, attachment.clone()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (slice_id, attachment) in &resolved {
            if !state.records.contains_key(slice_id) {
                return Err(DaemonError::LocalTransport {
                    operation: "slice.attach_agents",
                    message: format!("unknown slice `{}`", attachment.slice_ref),
                });
            }
        }
        let mut changed_slice_ids = Vec::new();
        for (slice_id, attachment) in resolved {
            let record = state
                .records
                .get_mut(&slice_id)
                .expect("slice existence should be preflighted");
            if !record
                .session_ids
                .iter()
                .any(|value| value == &attachment.session_id)
            {
                record.session_ids.push(attachment.session_id.clone());
            }
            record.session_id = Some(attachment.session_id);
            if !record
                .agent_ids
                .iter()
                .any(|value| value == &attachment.agent_id)
            {
                record.agent_ids.push(attachment.agent_id);
            }
            record.updated_at_ms = now_ms;
            if !changed_slice_ids.iter().any(|value| value == &slice_id) {
                changed_slice_ids.push(slice_id);
            }
        }
        Ok(changed_slice_ids
            .into_iter()
            .filter_map(|slice_id| state.records.get(&slice_id).cloned())
            .collect())
    }

    pub fn detach_agent(
        &self,
        slice_ref: &str,
        agent_id: &str,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
