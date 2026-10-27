use super::super::remote_kernel_selection::ensure_kernel_can_host_provider;
use crate::agent::RemoteAgentBinding;
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::slice::{SliceOperationGuard, SliceRecord, SliceStore};
use chariox_relay::protocol::RelayKernelPresence;

/// Recovery retains the recorded slice identity and Room admission through
/// lease creation and the home binding commit. Machine load is not placement.
pub(super) struct SliceBindingRecovery {
    record: SliceRecord,
    worker_kernel_id: String,
    worker_machine_id: String,
    _use_guard: Option<SliceOperationGuard>,
}

impl SliceBindingRecovery {
    pub(super) fn admit(
        slices: &SliceStore,
        binding: &RemoteAgentBinding,
        session_id: &str,
        operation: Option<&SliceOperationGuard>,
    ) -> Result<Option<Self>, DaemonError> {
        let mut matches = slices.list().into_iter().filter(|slice| {
            slice.worker_kernel_ref == binding.worker_kernel_id
                || slice.worker_kernel_id.as_deref() == Some(binding.worker_kernel_id.as_str())
        });
        let Some(record) = matches.next() else {
            if binding.worker_kernel_id.starts_with("slice:") || operation.is_some() {
                return Err(recovery_error("recorded slice worker has no slice record; restore or recreate that slice before retrying"));
            }
            return Ok(None);
        };
        if matches.next().is_some() {
            return Err(recovery_error(
                "recorded worker belongs to more than one slice",
            ));
        }
        if recorded_worker_id(&record) != binding.worker_kernel_id
            || record
                .worker_machine_id
                .as_deref()
                .is_some_and(|machine| machine != binding.worker_machine_id)
        {
            return Err(recovery_error(
                "observed worker disagrees with the persisted canonical slice identity",
            ));
        }
        let guard = match operation {
            Some(operation) => {
                operation.require_environment_use(slices, &record.id, Some(session_id))?;
                None
            }
            None => Some(slices.guard_environment_use(
                &record.id,
                Some(session_id),
                "slice.agent.recover",
            )?),
        };
        let record = slices.resolve(&record.id)?;
        if recorded_worker_id(&record) != binding.worker_kernel_id
            || record
                .worker_machine_id
                .as_deref()
                .is_some_and(|machine| machine != binding.worker_machine_id)
        {
            return Err(recovery_error(
                "recorded slice worker changed before recovery admission",
            ));
        }
        Ok(Some(Self {
            record,
            worker_kernel_id: binding.worker_kernel_id.clone(),
            worker_machine_id: binding.worker_machine_id.clone(),
            _use_guard: guard,
        }))
    }

    pub(super) fn select(
        &self,
        kernels: Vec<RelayKernelPresence>,
        provider: &str,
    ) -> Result<Option<RelayKernelPresence>, DaemonError> {
        kernels
            .into_iter()
            .find(|kernel| kernel.kernel_id == self.worker_kernel_id)
            .map(|kernel| self.require_worker(kernel, provider))
            .transpose()
    }

    pub(super) fn require_worker(
        &self,
        kernel: RelayKernelPresence,
        provider: &str,
    ) -> Result<RelayKernelPresence, DaemonError> {
        if kernel.kernel_id != self.worker_kernel_id || kernel.machine_id != self.worker_machine_id
        {
            return Err(recovery_error(
                "slice recovery must retain its recorded canonical worker and Machine",
            ));
        }
        ensure_kernel_can_host_provider(kernel, &self.worker_kernel_id, provider)
    }

    pub(super) fn unavailable(&self) -> DaemonError {
        recovery_error(&format!(
            "recorded slice worker `{}` is unavailable; restart the same slice before retrying",
            self.worker_kernel_id
        ))
    }

    pub(super) fn uses_connected_relay(&self, config: &DaemonConfig) -> bool {
        self.record.relay_endpoint.as_ref().is_some_and(|endpoint| {
            !endpoint.private && config.relay_url_uses_cloud_profile(&endpoint.url)
        })
    }

    pub(super) fn worktree_id(&self) -> String {
        self.record
            .development_publication
            .as_ref()
            .map(|publication| publication.primary_repository_path.clone())
            .unwrap_or_else(|| "/workspace".into())
    }
}

fn recovery_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "recover slice agent binding",
        message: message.into(),
    }
}

/// Local slice identity is stronger than a relay-advertised alias. Legacy
/// explicit/self-hosted references retain their already-recorded worker ID.
pub(super) fn recorded_slice_worker_id(
    slices: &SliceStore,
    worker_ref: &str,
) -> Result<Option<String>, DaemonError> {
    let mut matches = slices.list().into_iter().filter(|record| {
        record.worker_kernel_ref == worker_ref
            || record.worker_kernel_id.as_deref() == Some(worker_ref)
    });
    let Some(record) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        return Err(selection_error(
            "worker reference belongs to more than one local slice",
        ));
    }
    Ok(Some(recorded_worker_id(&record).to_string()))
}

pub(super) fn require_selected_slice_worker(
    expected: Option<&str>,
    kernel: RelayKernelPresence,
) -> Result<RelayKernelPresence, DaemonError> {
    if expected.is_some_and(|expected| kernel.kernel_id != expected) {
        return Err(selection_error(
            "relay metadata does not match the persisted canonical slice identity",
        ));
    }
    Ok(kernel)
}

fn recorded_worker_id(record: &SliceRecord) -> &str {
    if crate::slice::machine_scoped_slice_worker_ref(
        &record.worker_kernel_ref,
        &record.owner_machine_id,
    ) {
        &record.worker_kernel_ref
    } else {
        record
            .worker_kernel_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
            .unwrap_or(&record.worker_kernel_ref)
    }
}

fn selection_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "select slice worker",
        message: message.into(),
    }
}

/// Generic Machine failover cannot admit a Room slice. Reserved identities are
/// excluded even without a local record; exact custom SSH identities are also
/// excluded while recorded locally. Explicit slice selection has its own guard.
pub(super) fn ordinary_machine_workers(
    slices: &SliceStore,
    kernels: Vec<RelayKernelPresence>,
) -> Vec<RelayKernelPresence> {
    let records = slices.list();
    kernels
        .into_iter()
        .filter(|kernel| {
            !kernel.kernel_id.starts_with("slice:")
                && !records.iter().any(|record| {
                    record.worker_kernel_ref == kernel.kernel_id
                        || record.worker_kernel_id.as_deref() == Some(kernel.kernel_id.as_str())
                })
        })
        .collect()
}
