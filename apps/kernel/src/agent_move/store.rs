//! Move journals are private kernel state, independent of periodic Room snapshots.
use super::*;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::DaemonError;
use sha2::{Digest, Sha256};

#[derive(Default)]
pub(super) struct MoveRecords {
    pub homes: BTreeMap<String, HomeMoveOperation>,
    pub sources: BTreeMap<String, MoveSource>,
    pub targets: BTreeMap<String, MoveTarget>,
}

#[derive(Clone)]
pub(crate) struct AgentMoveStore {
    root: PathBuf,
    #[cfg(test)]
    crash_after: Arc<Mutex<Option<&'static str>>>,
    records: Arc<Mutex<MoveRecords>>,
    operations: Arc<Mutex<BTreeMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
}

impl AgentMoveStore {
    pub(crate) fn open(root: PathBuf) -> Result<Self, DaemonError> {
        fs::create_dir_all(&root).map_err(|_| {
            error("Cannot open the move journal; check kernel state permissions and free space")
        })?;
        let store = Self {
            root,
            #[cfg(test)]
            crash_after: Default::default(),
            records: Default::default(),
            operations: Default::default(),
        };
        let mut records = store.lock();
        for entry in fs::read_dir(&store.root)
            .map_err(|_| error("Cannot enumerate the move journal; repair kernel state"))?
        {
            let entry = entry
                .map_err(|_| error("Cannot enumerate the move journal; repair kernel state"))?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let role = if name.ends_with(".home.json") {
                "home"
            } else if name.ends_with(".source.json") {
                "source"
            } else if name.ends_with(".target.json") {
                "target"
            } else {
                continue;
            };
            if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                return Err(error(
                    "Move journal is not a regular file; repair kernel state",
                ));
            }
            let bytes = fs::read(&path)
                .map_err(|_| error("Cannot read the move journal; repair kernel state"))?;
            match role {
                "home" => {
                    let operation: HomeMoveOperation = decode(&bytes)?;
                    if path != store.record_path(&operation.identity.operation_id, role) {
                        return Err(error(
                            "Move journal identity differs from its filename; repair kernel state",
                        ));
                    }
                    records
                        .homes
                        .insert(operation.identity.operation_id.clone(), operation);
                }
                "source" => {
                    let source: MoveSource = decode(&bytes)?;
                    if path != store.record_path(&source.identity.operation_id, role) {
                        return Err(error("Move source journal identity differs from its filename; repair kernel state"));
                    }
                    records
                        .sources
                        .insert(source.identity.operation_id.clone(), source);
                }
                _ => {
                    let target: MoveTarget = decode(&bytes)?;
                    if path != store.record_path(&target.capture.identity.operation_id, role) {
                        return Err(error("Move target journal identity differs from its filename; repair kernel state"));
                    }
                    records
                        .targets
                        .insert(target.capture.identity.operation_id.clone(), target);
                }
            }
        }
        drop(records);
        Ok(store)
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, MoveRecords> {
        self.records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn record_path(&self, operation: &str, role: &str) -> PathBuf {
        self.root.join(format!(
            "{:x}.{role}.json",
            Sha256::digest(operation.as_bytes())
        ))
    }

    pub(crate) fn artifact_root(&self, operation: &str) -> PathBuf {
        self.root
            .join(format!("{:x}", Sha256::digest(operation.as_bytes())))
    }

    pub(crate) fn home(&self, operation: &str) -> Option<HomeMoveOperation> {
        self.lock().homes.get(operation).cloned()
    }

    pub(crate) fn pending(&self) -> Vec<HomeMoveOperation> {
        self.lock()
            .homes
            .values()
            .filter(|operation| operation.phase.is_pending())
            .cloned()
            .collect()
    }

    pub(crate) fn for_agent(&self, session: &str, agent: &str) -> Option<HomeMoveOperation> {
        self.lock()
            .homes
            .values()
            .filter(|operation| {
                operation.identity.home_session_id == session
                    && operation.identity.home_agent_id == agent
            })
            .max_by_key(|operation| operation.identity.generation)
            .cloned()
    }

    pub(crate) fn save_home(
        &self,
        mut operation: HomeMoveOperation,
    ) -> Result<HomeMoveOperation, DaemonError> {
        let mut records = self.lock();
        if let Some(previous) = records.homes.get(&operation.identity.operation_id) {
            if previous.identity != operation.identity || previous.revision != operation.revision {
                return Err(error(
                    "Move generation changed; query the current operation before retrying",
                ));
            }
        } else if operation.revision != 0 {
            return Err(error(
                "Move journal is missing; repair kernel state before retrying",
            ));
        }
        operation.revision = operation
            .revision
            .checked_add(1)
            .ok_or_else(|| error("Move revision exhausted"))?;
        atomic_save(
            &self.record_path(&operation.identity.operation_id, "home"),
            &operation,
        )?;
        records
            .homes
            .insert(operation.identity.operation_id.clone(), operation.clone());
        drop(records);
        #[cfg(test)]
        self.crash_at_boundary_for_test(&operation.phase);
        Ok(operation)
    }

    pub(crate) fn source(&self, operation: &str) -> Option<MoveSource> {
        self.lock().sources.get(operation).cloned()
    }

    pub(crate) fn save_source(&self, source: MoveSource) -> Result<(), DaemonError> {
        let mut records = self.lock();
        if records
            .sources
            .get(&source.identity.operation_id)
            .is_some_and(|old| {
                old.identity != source.identity
                    || old.session_id != source.session_id
                    || old.agent_id != source.agent_id
            })
        {
            return Err(error(
                "Move source identity changed; query the original operation",
            ));
        }
        atomic_save(
            &self.record_path(&source.identity.operation_id, "source"),
            &source,
        )?;
        records
            .sources
            .insert(source.identity.operation_id.clone(), source);
        Ok(())
    }

    pub(crate) fn target(&self, operation: &str) -> Option<MoveTarget> {
        self.lock().targets.get(operation).cloned()
    }

    pub(crate) fn save_target(&self, target: MoveTarget) -> Result<(), DaemonError> {
        let mut records = self.lock();
        if records
            .targets
            .get(&target.capture.identity.operation_id)
            .is_some_and(|old| {
                old.capture != target.capture || old.destination_root != target.destination_root
            })
        {
            return Err(error(
                "Move target identity changed; query the original operation",
            ));
        }
        atomic_save(
            &self.record_path(&target.capture.identity.operation_id, "target"),
            &target,
        )?;
        records
            .targets
            .insert(target.capture.identity.operation_id.clone(), target);
        Ok(())
    }

    pub(crate) fn dispatch_fenced(&self, session: &str, agent: &str) -> bool {
        let records = self.lock();
        records.homes.values().any(|operation| {
            operation.identity.home_session_id == session
                && operation.identity.home_agent_id == agent
                && operation.phase.is_pending()
        }) || records.sources.values().any(|source| {
            source.session_id == session
                && source.agent_id == agent
                && !source.retired
                && !source.released
        }) || records.targets.values().any(|target| {
            target.backing_session_id.as_deref() == Some(session)
                && target.backing_agent_id.as_deref() == Some(agent)
                && !target.active
                && !target.retired
        })
    }
}

fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, DaemonError> {
    serde_json::from_slice(bytes).map_err(|_| {
        error("Move journal is invalid; repair kernel state before dispatching this agent")
    })
}

pub(super) fn atomic_save(path: &Path, value: &impl Serialize) -> Result<(), DaemonError> {
    let parent = path
        .parent()
        .ok_or_else(|| error("Move journal has no parent"))?;
    let temporary = parent.join(format!(".move-{:016x}.tmp", rand::random::<u64>()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|_| {
            error("Cannot create move journal; check free space and kernel state permissions")
        })?;
        serde_json::to_writer(&mut file, value).map_err(|_| error("Cannot encode move journal"))?;
        file.write_all(b"\n")
            .and_then(|_| file.sync_all())
            .map_err(|_| error("Cannot sync move journal; check free space"))?;
        fs::rename(&temporary, path)
            .map_err(|_| error("Cannot publish move journal; check kernel state permissions"))?;
        File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|_| error("Cannot sync move journal directory"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

impl std::fmt::Debug for AgentMoveStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentMoveStore")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl AgentMoveStore {
    pub(crate) fn target_for_agent(&self, session: &str, agent: &str) -> Option<MoveTarget> {
        self.lock()
            .targets
            .values()
            .filter(|target| {
                !target.retired
                    && target.backing_session_id.as_deref() == Some(session)
                    && target.backing_agent_id.as_deref() == Some(agent)
            })
            .max_by_key(|target| target.capture.identity.generation)
            .cloned()
    }

    pub(crate) fn checkpoint_for_launch(
        &self,
        request: &crate::provider::LaunchProviderRequest,
    ) -> Result<Option<crate::provider::ProviderNativeCheckpoint>, DaemonError> {
        let Some(agent) = request.agent_id.as_deref() else {
            return Ok(None);
        };
        let Some(target) = self.target_for_agent(&request.session_id, agent) else {
            return Ok(None);
        };
        if request
            .resume_state
            .as_ref()
            .and_then(|state| state.required_native_session())
            .is_none()
        {
            return Ok(None);
        }
        self.native_checkpoint(&target.capture)
    }
}

impl AgentMoveStore {
    pub(crate) fn with_admission<T>(
        &self,
        session: &str,
        agent: Option<&str>,
        preparation: Option<&str>,
        action: impl FnOnce() -> Result<T, DaemonError>,
    ) -> Result<T, DaemonError> {
        let records = self.lock();
        if let Some(agent) = agent {
            let preparing_target = preparation.is_some_and(|operation| {
                records.targets.get(operation).is_some_and(|target| {
                    target.receipt.is_some()
                        && target.backing_session_id.as_deref() == Some(session)
                        && target.backing_agent_id.as_deref() == Some(agent)
                })
            });
            let fenced = records.homes.values().any(|operation| {
                operation.identity.home_session_id == session
                    && operation.identity.home_agent_id == agent
                    && operation.phase.is_pending()
            }) || records.sources.values().any(|source| {
                source.session_id == session
                    && source.agent_id == agent
                    && !source.released
                    && (!source.retired
                        || !records.targets.values().any(|target| {
                            !target.retired
                                && target.active
                                && target.capture.identity.generation > source.identity.generation
                                && target.backing_session_id.as_deref() == Some(session)
                                && target.backing_agent_id.as_deref() == Some(agent)
                        }))
            }) || records.targets.values().any(|target| {
                target.backing_session_id.as_deref() == Some(session)
                    && target.backing_agent_id.as_deref() == Some(agent)
                    && !target.active
                    && !target.retired
            });
            if fenced && !preparing_target {
                return Err(error("Agent move holds provider admission. Finish or roll back the durable move before retrying"));
            }
        }
        let result = action();
        drop(records);
        result
    }
}

impl AgentMoveStore {
    pub(crate) fn execution_session(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &str,
    ) -> crate::session::RuntimeSession {
        self.target_for_agent(session.id(), agent)
            .and_then(|target| target.receipt)
            .and_then(|receipt| {
                receipt.primary_path().ok().map(|path| {
                    session.for_execution_project(
                        &receipt.target_project_id,
                        &path.display().to_string(),
                        &path.display().to_string(),
                    )
                })
            })
            .unwrap_or_else(|| session.clone())
    }
}

impl AgentMoveStore {
    pub(crate) async fn operation(&self, key: String) -> tokio::sync::OwnedMutexGuard<()> {
        let operation = {
            let mut operations = self
                .operations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            operations.retain(|_, operation| operation.strong_count() > 0);
            match operations.get(&key).and_then(std::sync::Weak::upgrade) {
                Some(operation) => operation,
                None => {
                    let operation = Arc::new(tokio::sync::Mutex::new(()));
                    operations.insert(key, Arc::downgrade(&operation));
                    operation
                }
            }
        };
        operation.lock_owned().await
    }
}

impl AgentMoveStore {
    pub(crate) fn targets(&self) -> Vec<MoveTarget> {
        self.lock().targets.values().cloned().collect()
    }

    pub(crate) fn retire_backing(&self, leased_agent_id: &str) -> Result<(), DaemonError> {
        for mut target in self.targets().into_iter().filter(|target| {
            target
                .backing
                .as_ref()
                .is_some_and(|backing| backing.leased_agent.id == leased_agent_id)
        }) {
            target.retired = true;
            self.save_target(target)?;
        }
        Ok(())
    }
}

#[cfg(test)]
impl AgentMoveStore {
    pub(crate) fn crash_after_phase_for_test(&self, phase: &'static str) {
        *self.crash_after.lock().unwrap() = Some(phase);
    }

    fn crash_at_boundary_for_test(&self, phase: &HomeMovePhase) {
        let name = match phase {
            HomeMovePhase::Fenced => "fenced",
            HomeMovePhase::SourceStopped => "source_stopped",
            HomeMovePhase::Captured { .. } => "captured",
            HomeMovePhase::Materialized { .. } => "materialized",
            HomeMovePhase::Bound { .. } => "bound",
            HomeMovePhase::Committed { .. } => "committed",
            HomeMovePhase::Complete { .. } => "complete",
            HomeMovePhase::RolledBack { .. } => "rolled_back",
        };
        let crash = {
            let mut requested = self.crash_after.lock().unwrap();
            if *requested == Some(name) {
                requested.take();
                true
            } else {
                false
            }
        };
        if crash {
            panic!("simulated kernel crash after durable Move boundary: {name}");
        }
    }
}
