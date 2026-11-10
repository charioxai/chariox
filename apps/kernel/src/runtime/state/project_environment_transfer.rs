//! Import and Update across kernels: this kernel asks another kernel of the same user for what the
//! Environment lists, and answers the same questions about its own Projects.
use super::*;
use crate::local::{
    EnvironmentEntryKind, EnvironmentOperationKind, EnvironmentOperationState,
    EnvironmentOperationView, ImportProjectEnvironmentEntriesRequest,
};
use crate::project_environment::{
    environment_error, read_file_slice, register_environment_files, run_import, source_holdings,
    Begun, EnvironmentSource, FileSlice, ImportTarget, Registration, RegistrationHost, SourceError,
    SourceSnapshot, UserRegistrations, MAX_IMPORT_FILE_BYTES,
};
use crate::transport::relay_peer::{
    RelayPeerRequest, RelayPeerResponse, RemoteCredentialSecretInput, RemoteFileBytes,
    RemoteProjectRegistration,
};
use std::path::PathBuf;
use zeroize::Zeroizing;

/// A File travels in one message, so the wait covers the largest one over a slow uplink.
const PEER_TIMEOUT: Duration = Duration::from_secs(90);

/// The kernel the entries come from, reached over the relay.
struct PeerSource {
    runtime: KernelRuntimeState,
    owner: String,
    kernel: String,
}

impl PeerSource {
    async fn ask(&self, request: RelayPeerRequest) -> Result<RelayPeerResponse, SourceError> {
        match self
            .runtime
            .send_same_owner_peer_request(&self.owner, &self.kernel, request, PEER_TIMEOUT)
            .await
        {
            Ok(RelayPeerResponse::ProjectEnvironmentRefused { reason }) => {
                Err(SourceError::Refused(reason))
            }
            Ok(response) => Ok(response),
            Err(_) => Err(SourceError::Offline),
        }
    }
}

impl EnvironmentSource for PeerSource {
    async fn read(&self, project_id: &str) -> Result<SourceSnapshot, SourceError> {
        match self
            .ask(RelayPeerRequest::ReadProjectEnvironment {
                project_id: project_id.into(),
            })
            .await?
        {
            RelayPeerResponse::ProjectEnvironmentRead {
                source_label,
                record,
                secrets,
                files,
                mcps,
                skills,
            } => Ok(SourceSnapshot {
                label: source_label,
                record,
                secrets,
                files,
                mcps,
                skills,
            }),
            _ => Err(SourceError::Refused(
                "The source answered with something unexpected".into(),
            )),
        }
    }

    async fn pull_secret(
        &self,
        project_id: &str,
        name: &str,
    ) -> Result<Option<Zeroizing<String>>, SourceError> {
        match self
            .ask(RelayPeerRequest::PullProjectEnvironmentSecret {
                project_id: project_id.into(),
                name: name.into(),
            })
            .await?
        {
            RelayPeerResponse::ProjectEnvironmentSecretPulled { value } => {
                Ok(value.map(RemoteCredentialSecretInput::into_zeroizing))
            }
            _ => Err(SourceError::Refused(
                "The source answered with something unexpected".into(),
            )),
        }
    }

    async fn pull_registration(
        &self,
        project_id: &str,
        kind: EnvironmentEntryKind,
        name: &str,
    ) -> Result<Option<Registration>, SourceError> {
        match self
            .ask(RelayPeerRequest::PullProjectEnvironmentRegistration {
                project_id: project_id.into(),
                entry_kind: kind,
                name: name.into(),
            })
            .await?
        {
            RelayPeerResponse::ProjectEnvironmentRegistrationPulled { registration } => {
                Ok(registration.map(|registration| registration.0))
            }
            _ => Err(SourceError::Refused(
                "The source answered with something unexpected".into(),
            )),
        }
    }

    async fn pull_file(
        &self,
        project_id: &str,
        path: &str,
        offset: u64,
    ) -> Result<Option<FileSlice>, SourceError> {
        match self
            .ask(RelayPeerRequest::PullProjectEnvironmentFile {
                project_id: project_id.into(),
                path: path.into(),
                offset,
            })
            .await?
        {
            RelayPeerResponse::ProjectEnvironmentFilePulled {
                size_bytes,
                sha256,
                content,
            } => content
                .map(|content| {
                    content
                        .decode()
                        .zip(sha256)
                        .map(|(bytes, sha256)| FileSlice {
                            size_bytes,
                            sha256,
                            bytes,
                        })
                        .ok_or_else(|| SourceError::Refused("The file arrived damaged".into()))
                })
                .transpose(),
            _ => Err(SourceError::Refused(
                "The source answered with something unexpected".into(),
            )),
        }
    }
}

impl KernelRuntimeState {
    /// The newest state of an operation, whichever engine runs it.
    pub(super) fn environment_operation(
        &self,
        operation_id: &str,
    ) -> Result<Option<EnvironmentOperationView>, DaemonError> {
        if let Some(operation) = self.owned.project_environment_setups.get(operation_id)? {
            return Ok(Some(operation));
        }
        self.owned.environment_operations.get(operation_id)
    }

    pub(super) fn cancel_environment_operation_by_id(
        &self,
        operation_id: &str,
    ) -> Result<EnvironmentOperationView, DaemonError> {
        let operation = if self
            .owned
            .project_environment_setups
            .get(operation_id)?
            .is_some()
        {
            self.owned.project_environment_setups.cancel(operation_id)?
        } else {
            self.owned
                .environment_operations
                .cancel(operation_id)?
                .ok_or_else(|| environment_error("this operation is unknown"))?
        };
        Ok(operation)
    }

    pub(crate) fn import_project_environment_entries(
        &self,
        request: ImportProjectEnvironmentEntriesRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let project = self.owned_project(&request.project_id, caller_user_id)?;
        let checkout = self.project_checkout(&project, request.checkout_path.as_deref())?;
        crate::managed_context::target_vault::require_open_vault_for_secret_entries(
            &self.owned.config_projection.snapshot(),
            request.entries.as_deref(),
        )?;
        let view = EnvironmentOperationView {
            operation_id: request.operation_id,
            project_id: project.id().into(),
            checkout_path: checkout.to_string_lossy().into_owned(),
            kind: EnvironmentOperationKind::Import,
            state: EnvironmentOperationState::Running,
            started_at_ms: crate::session::unix_epoch_ms(),
            ended_at_ms: None,
            exit_code: None,
            reason: None,
            updates: 0,
            output_tail: String::new(),
            entries: Vec::new(),
            source_label: None,
        };
        let handle = match self.owned.environment_operations.begin(view.clone())? {
            Begun::Existing(operation) => {
                return Ok(LocalDaemonResponse::EnvironmentOperation { operation })
            }
            Begun::Started(handle) => handle,
        };
        let runtime = self.clone();
        let project_id = project.id().to_string();
        let checkouts: Vec<PathBuf> = project.workspace_ids().iter().map(PathBuf::from).collect();
        let source = PeerSource {
            runtime: self.clone(),
            owner: caller_user_id.into(),
            kernel: request.source_kernel_id,
        };
        let entries = request.entries;
        tokio::spawn(async move {
            let config = runtime.owned.config_projection.snapshot();
            let vault = match crate::secret::project_environment_vault(&config) {
                Ok(vault) => vault,
                Err(error) => {
                    handle.finish(EnvironmentOperationState::Failed, Some(error.to_string()));
                    return;
                }
            };
            let store = runtime.environment_store();
            let on_record = move |record: &crate::project_environment::ProjectEnvironmentRecord| {
                register_environment_files(&checkouts, &record.files)
            };
            run_import(
                handle,
                &source,
                ImportTarget {
                    project_id: &project_id,
                    checkout: &checkout,
                    store: &store,
                    vault: vault.as_ref(),
                    registrations: &UserRegistrations,
                    on_record: &on_record,
                },
                entries,
            )
            .await;
        });
        Ok(LocalDaemonResponse::EnvironmentOperation { operation: view })
    }

    /// The source side: what this kernel hands to another kernel of the same user.
    pub(crate) fn receive_project_environment_peer(
        &self,
        owner: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let refuse = |reason: &str| -> Result<RelayPeerResponse, DaemonError> {
            Ok(RelayPeerResponse::ProjectEnvironmentRefused {
                reason: reason.into(),
            })
        };
        let config = self.owned.config_projection.snapshot();
        let label = Self::machine_label(&config);
        let (project_id, secret, file, registration) = match &request {
            RelayPeerRequest::ReadProjectEnvironment { project_id } => {
                (project_id, None, None, None)
            }
            RelayPeerRequest::PullProjectEnvironmentSecret { project_id, name } => {
                (project_id, Some(name), None, None)
            }
            RelayPeerRequest::PullProjectEnvironmentFile {
                project_id,
                path,
                offset,
            } => (project_id, None, Some((path, *offset)), None),
            RelayPeerRequest::PullProjectEnvironmentRegistration {
                project_id,
                entry_kind,
                name,
            } => (project_id, None, None, Some((*entry_kind, name))),
            _ => return refuse("not a Project Environment request"),
        };
        let Ok(project) = self.owned_project(project_id, owner) else {
            return refuse(&format!("{label} does not hold this Project"));
        };
        let checkout = self.project_checkout(&project, None)?;
        let store = self.environment_store();
        let vault = crate::secret::project_environment_vault(&config)?;
        let Some(record) = store.load(project.id())? else {
            return match &request {
                RelayPeerRequest::ReadProjectEnvironment { .. } => {
                    Ok(RelayPeerResponse::ProjectEnvironmentRead {
                        source_label: label,
                        record: None,
                        secrets: Vec::new(),
                        files: Vec::new(),
                        mcps: Vec::new(),
                        skills: Vec::new(),
                    })
                }
                _ => refuse(&format!("{label} has no Environment for this Project")),
            };
        };
        if let Some(name) = secret {
            if !record.secrets.contains(name) {
                return refuse(&format!("{name} is not in the Environment of {label}"));
            }
            return match vault.find_secret(
                &crate::project_environment::project_environment_vault_service(project.id()),
                name,
            ) {
                Ok(value) => Ok(RelayPeerResponse::ProjectEnvironmentSecretPulled {
                    value: value.map(|value| {
                        RemoteCredentialSecretInput::from_zeroizing(Zeroizing::new(value))
                    }),
                }),
                Err(error) if crate::secret::is_chariox_vault_locked_error(&error) => {
                    refuse(&format!("The Vault of {label} is locked"))
                }
                Err(_) => refuse(&format!("The Vault of {label} could not be read")),
            };
        }
        if let Some((kind, name)) = registration {
            let listed = match kind {
                EnvironmentEntryKind::Mcp => &record.mcps,
                EnvironmentEntryKind::Skill => &record.skills,
                _ => return refuse("not a registration"),
            };
            if !listed
                .iter()
                .any(|entry| &entry.name == name && entry.file_path().is_none())
            {
                return refuse(&format!("{name} is not in the Environment of {label}"));
            }
            return match UserRegistrations.read(kind, name) {
                Ok(registration) => Ok(RelayPeerResponse::ProjectEnvironmentRegistrationPulled {
                    registration: registration.map(RemoteProjectRegistration),
                }),
                Err(reason) => refuse(&reason),
            };
        }
        if let Some((path, offset)) = file {
            if !record.files.contains(path) {
                return refuse(&format!("{path} is not in the Environment of {label}"));
            }
            let full = checkout.join(path);
            let absent = || {
                Ok(RelayPeerResponse::ProjectEnvironmentFilePulled {
                    size_bytes: 0,
                    sha256: None,
                    content: None,
                })
            };
            let Ok(metadata) = std::fs::symlink_metadata(&full) else {
                return absent();
            };
            if !metadata.is_file() {
                return absent();
            }
            if metadata.len() > MAX_IMPORT_FILE_BYTES {
                return refuse("The file is too large to import");
            }
            let Ok(slice) = read_file_slice(&full, offset) else {
                return refuse(&format!("{path} could not be read on {label}"));
            };
            return Ok(RelayPeerResponse::ProjectEnvironmentFilePulled {
                size_bytes: slice.size_bytes,
                sha256: Some(slice.sha256),
                content: Some(RemoteFileBytes::encode(&slice.bytes)),
            });
        }
        let held = source_holdings(
            &store,
            project.id(),
            vault.as_ref(),
            &UserRegistrations,
            &checkout,
            label,
        )?;
        Ok(RelayPeerResponse::ProjectEnvironmentRead {
            source_label: held.label,
            record: held.record,
            secrets: held.secrets,
            files: held.files,
            mcps: held.mcps,
            skills: held.skills,
        })
    }
}
