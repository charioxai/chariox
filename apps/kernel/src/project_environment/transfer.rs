//! Import: brings the entries this kernel lacks from another kernel of the same user. The record
//! comes first when the source's revision is newer, then each Secret value and File one at a time,
//! so every row has its own result and Stop keeps what already landed.
use super::operations::OperationHandle;
use super::{
    environment_error, project_environment_vault_service, relative_project_path, ExtensionEntry,
    FilePresence, ProjectEnvironmentRecord, ProjectEnvironmentStore, Registration,
    RegistrationHost,
};
use crate::error::DaemonError;
use crate::local::{
    EnvironmentEntryKind, EnvironmentEntryRef, EnvironmentOperationEntry,
    EnvironmentOperationEntryState, EnvironmentOperationState,
};
use crate::secret::CredentialVaultStore;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::future::Future;
use std::path::Path;
use zeroize::Zeroizing;

/// A File travels in slices of this size: one peer message is encoded twice on the way and must stay under the relay's bound.
pub(crate) const IMPORT_CHUNK_BYTES: u64 = 4 * 1024 * 1024;
/// Slices requested ahead of the one being written.
const SLICES_IN_FLIGHT: usize = 3;
pub(crate) const MAX_IMPORT_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_IMPORT_TOTAL_BYTES: u64 = 128 * 1024 * 1024;
pub(crate) const SOURCE_OFFLINE: &str = "Source offline";

/// What the source holds, without any value.
#[derive(Clone)]
pub(crate) struct SourceSnapshot {
    pub label: String,
    pub record: Option<ProjectEnvironmentRecord>,
    /// Secret names the source's Vault has a value for.
    pub secrets: Vec<String>,
    /// Files that exist in the source's checkout.
    pub files: Vec<String>,
    /// MCP servers and skills the source has registered, of those the record lists as registrations.
    pub mcps: Vec<String>,
    pub skills: Vec<String>,
}

/// One slice of a source file, with the size and SHA-256 of the whole file it was read from.
pub(crate) struct FileSlice {
    pub size_bytes: u64,
    /// Every slice names the content it belongs to; the receiver publishes the file only when
    /// what it assembled has this digest.
    pub sha256: String,
    pub bytes: Zeroizing<Vec<u8>>,
}

/// Up to one slice of the file at `offset`, with the size and digest of the whole file read
/// through the same open file.
pub(crate) fn read_file_slice(path: &Path, offset: u64) -> std::io::Result<FileSlice> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let size_bytes = file.metadata()?.len();
    let mut hash = Sha256::new();
    std::io::copy(&mut (&mut file).take(size_bytes), &mut hash)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(IMPORT_CHUNK_BYTES).read_to_end(&mut bytes)?;
    Ok(FileSlice {
        size_bytes,
        sha256: format!("{:x}", hash.finalize()),
        bytes,
    })
}

/// Why the source gave nothing: it could not be reached, or it answered that it will not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SourceError {
    Offline,
    Refused(String),
}

impl SourceError {
    fn reason(&self) -> String {
        match self {
            Self::Offline => SOURCE_OFFLINE.into(),
            Self::Refused(reason) => reason.clone(),
        }
    }
}

/// The kernel the entries come from.
pub(crate) trait EnvironmentSource: Send + Sync {
    fn read(
        &self,
        project_id: &str,
    ) -> impl Future<Output = Result<SourceSnapshot, SourceError>> + Send;
    fn pull_secret(
        &self,
        project_id: &str,
        name: &str,
    ) -> impl Future<Output = Result<Option<Zeroizing<String>>, SourceError>> + Send;
    /// The slice of a file at `offset`, or `None` when the source has no such file.
    fn pull_file(
        &self,
        project_id: &str,
        path: &str,
        offset: u64,
    ) -> impl Future<Output = Result<Option<FileSlice>, SourceError>> + Send;
    /// The registration of an MCP or skill entry, or `None` when the source has none of that name.
    fn pull_registration(
        &self,
        project_id: &str,
        kind: EnvironmentEntryKind,
        name: &str,
    ) -> impl Future<Output = Result<Option<Registration>, SourceError>> + Send;
}

/// Where the entries land on this kernel.
pub(crate) struct ImportTarget<'a> {
    pub project_id: &'a str,
    pub checkout: &'a Path,
    pub store: &'a ProjectEnvironmentStore,
    pub vault: &'a dyn CredentialVaultStore,
    pub registrations: &'a dyn RegistrationHost,
    /// Called with the record after a newer one was adopted, so the Files rules follow it.
    pub on_record: &'a (dyn Fn(&ProjectEnvironmentRecord) + Send + Sync),
}

fn secret_here(target: &ImportTarget<'_>, name: &str) -> Result<bool, DaemonError> {
    Ok(target
        .vault
        .find_secret(&project_environment_vault_service(target.project_id), name)?
        .is_some())
}

fn file_here(target: &ImportTarget<'_>, path: &str) -> bool {
    matches!(
        super::file_presence(target.checkout, path),
        FilePresence::Present { .. }
    )
}

fn entry(kind: EnvironmentEntryKind, name: &str) -> EnvironmentOperationEntry {
    EnvironmentOperationEntry {
        kind,
        name: name.into(),
        state: EnvironmentOperationEntryState::Pending,
        reason: None,
    }
}

fn set_entry(
    handle: &OperationHandle,
    index: usize,
    state: EnvironmentOperationEntryState,
    reason: Option<String>,
) {
    handle.update(|view| {
        if let Some(entry) = view.entries.get_mut(index) {
            entry.state = state;
            entry.reason = reason;
        }
    });
}

/// A File arrives into a private temporary file next to its destination and appears under its name
/// only when complete; a file that appeared meanwhile is never replaced.
struct FileWriter {
    temporary: std::path::PathBuf,
    destination: std::path::PathBuf,
    file: Option<std::fs::File>,
}

impl FileWriter {
    fn create(checkout: &Path, path: &str) -> Result<Self, String> {
        use std::os::unix::fs::OpenOptionsExt;
        relative_project_path(path).map_err(str::to_owned)?;
        let destination = checkout.join(path);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|_| "A folder could not be created".to_owned())?;
            // A folder of the checkout that is a link out of it is not a place to write.
            let inside = parent.canonicalize().ok().zip(checkout.canonicalize().ok());
            if !inside.is_some_and(|(parent, checkout)| parent.starts_with(checkout)) {
                return Err("The file would land outside the project".to_owned());
            }
        }
        let temporary =
            destination.with_file_name(format!(".chariox-import-{}", rand::random::<u64>()));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| "The file could not be written".to_owned())?;
        Ok(Self {
            temporary,
            destination,
            file: Some(file),
        })
    }

    fn append(&mut self, bytes: &[u8]) -> Result<(), String> {
        use std::io::Write;
        self.file
            .as_mut()
            .expect("an open writer")
            .write_all(bytes)
            .map_err(|_| "The file could not be written".to_owned())
    }

    fn finish(mut self) -> Result<(), String> {
        let file = self.file.take().expect("an open writer");
        file.sync_all()
            .map_err(|_| "The file could not be written".to_owned())?;
        // `hard_link` refuses to replace a file that appeared meanwhile.
        std::fs::hard_link(&self.temporary, &self.destination).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                "Already present here".to_owned()
            } else {
                "The file could not be written".to_owned()
            }
        })
    }
}

impl Drop for FileWriter {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.temporary);
    }
}

/// The entries to import: those asked for, or every one of the record this kernel is missing.
/// Whether the Environment lists `name` as a registration.
fn registered(entries: &[ExtensionEntry], name: &str) -> bool {
    entries
        .iter()
        .any(|entry| entry.name == name && entry.file_path().is_none())
}

fn plan(
    record: &ProjectEnvironmentRecord,
    requested: Option<&[EnvironmentEntryRef]>,
    target: &ImportTarget<'_>,
) -> Vec<(EnvironmentOperationEntry, Option<String>)> {
    let mut entries = Vec::new();
    match requested {
        Some(requested) => {
            for wanted in requested {
                let listed = match wanted.kind {
                    EnvironmentEntryKind::Secret => record.secrets.contains(&wanted.name),
                    EnvironmentEntryKind::File => record.files.contains(&wanted.name),
                    EnvironmentEntryKind::Mcp => registered(&record.mcps, &wanted.name),
                    EnvironmentEntryKind::Skill => registered(&record.skills, &wanted.name),
                };
                let refusal = (!listed).then(|| "Not in this Environment".to_owned());
                entries.push((entry(wanted.kind, &wanted.name), refusal));
            }
        }
        None => {
            for name in &record.secrets {
                // A locked Vault cannot tell, so the row is tried and says why it failed.
                if !secret_here(target, name).unwrap_or(false) {
                    entries.push((entry(EnvironmentEntryKind::Secret, name), None));
                }
            }
            for path in &record.files {
                if !file_here(target, path) {
                    entries.push((entry(EnvironmentEntryKind::File, path), None));
                }
            }
            for (kind, listed) in [
                (EnvironmentEntryKind::Mcp, &record.mcps),
                (EnvironmentEntryKind::Skill, &record.skills),
            ] {
                for registration in listed.iter().filter(|e| e.file_path().is_none()) {
                    if !target.registrations.has(kind, &registration.name) {
                        entries.push((entry(kind, &registration.name), None));
                    }
                }
            }
        }
    }
    entries
}

fn finish(handle: &OperationHandle) {
    let (failed, first_reason) = {
        let Some(view) = handle.snapshot() else {
            return;
        };
        let failed = view
            .entries
            .iter()
            .filter(|entry| entry.state == EnvironmentOperationEntryState::Failed)
            .count();
        let reason = view.entries.iter().find_map(|entry| entry.reason.clone());
        (failed, reason)
    };
    if failed == 0 {
        handle.finish(EnvironmentOperationState::Succeeded, None);
    } else {
        handle.finish(EnvironmentOperationState::Failed, first_reason);
    }
}

/// Why an import that selects a Secret does not start while this kernel's Vault is closed.
const VAULT_LOCKED_REFUSAL: &str =
    "This kernel's Vault is locked; unlock it and try again. Nothing was imported.";

/// Whether the entries this import would bring include a Secret that cannot be written because the
/// Vault is closed. Checked on the selection resolved against the record, before anything lands.
fn secret_selected_while_vault_locked(
    record: &ProjectEnvironmentRecord,
    requested: Option<&[EnvironmentEntryRef]>,
    target: &ImportTarget<'_>,
) -> bool {
    plan(record, requested, target)
        .iter()
        .filter(|(entry, _)| entry.kind == EnvironmentEntryKind::Secret)
        .any(|(entry, _)| {
            matches!(
                secret_here(target, &entry.name),
                Err(error) if crate::secret::is_chariox_vault_locked_error(&error)
            )
        })
}

pub(crate) async fn run_import<S: EnvironmentSource>(
    handle: OperationHandle,
    source: &S,
    target: ImportTarget<'_>,
    requested: Option<Vec<EnvironmentEntryRef>>,
) {
    let snapshot = tokio::select! {
        snapshot = source.read(target.project_id) => snapshot,
        _ = handle.stopped() => return,
    };
    let snapshot = match snapshot {
        Ok(snapshot) => snapshot,
        Err(error) => {
            handle.finish(EnvironmentOperationState::Failed, Some(error.reason()));
            return;
        }
    };
    handle.update(|view| view.source_label = Some(snapshot.label.clone()));
    let local = match target.store.load(target.project_id) {
        Ok(local) => local,
        Err(error) => {
            handle.finish(EnvironmentOperationState::Failed, Some(error.to_string()));
            return;
        }
    };
    let local_revision = local.as_ref().map_or(0, |record| record.revision);
    let newer = snapshot
        .record
        .as_ref()
        .filter(|source_record| source_record.revision > local_revision);
    if let Some(record) = newer.or(local.as_ref()) {
        if secret_selected_while_vault_locked(record, requested.as_deref(), &target) {
            handle.finish(
                EnvironmentOperationState::Failed,
                Some(VAULT_LOCKED_REFUSAL.into()),
            );
            return;
        }
    }
    let record = match snapshot.record.clone() {
        Some(source_record) if source_record.revision > local_revision => {
            match target.store.adopt(target.project_id, source_record.clone()) {
                Ok(()) => {
                    (target.on_record)(&source_record);
                    source_record
                }
                Err(error) => {
                    handle.finish(EnvironmentOperationState::Failed, Some(error.to_string()));
                    return;
                }
            }
        }
        _ => match local {
            Some(local) => local,
            None => {
                handle.finish(
                    EnvironmentOperationState::Failed,
                    Some(format!(
                        "{} has no Environment for this Project",
                        snapshot.label
                    )),
                );
                return;
            }
        },
    };
    let planned = plan(&record, requested.as_deref(), &target);
    handle.update(|view| {
        view.entries = planned.iter().map(|(entry, _)| entry.clone()).collect();
    });
    let mut landed_bytes = 0_u64;
    let mut offline = false;
    for (index, (entry, refusal)) in planned.iter().enumerate() {
        if handle.is_stopped() {
            return;
        }
        if let Some(reason) = refusal {
            set_entry(
                &handle,
                index,
                EnvironmentOperationEntryState::Failed,
                Some(reason.clone()),
            );
            continue;
        }
        if offline {
            set_entry(
                &handle,
                index,
                EnvironmentOperationEntryState::Failed,
                Some(SOURCE_OFFLINE.into()),
            );
            continue;
        }
        set_entry(
            &handle,
            index,
            EnvironmentOperationEntryState::Importing,
            None,
        );
        let outcome = tokio::select! {
            outcome = import_entry(source, &snapshot, &target, entry, &mut landed_bytes) => outcome,
            _ = handle.stopped() => return,
        };
        match outcome {
            Ok(()) => set_entry(
                &handle,
                index,
                EnvironmentOperationEntryState::Imported,
                None,
            ),
            Err(ImportFailure::Offline) => {
                offline = true;
                set_entry(
                    &handle,
                    index,
                    EnvironmentOperationEntryState::Failed,
                    Some(SOURCE_OFFLINE.into()),
                );
            }
            Err(ImportFailure::Entry(reason)) => {
                set_entry(
                    &handle,
                    index,
                    EnvironmentOperationEntryState::Failed,
                    Some(reason),
                );
            }
        }
    }
    finish(&handle);
}

enum ImportFailure {
    Offline,
    Entry(String),
}

impl From<SourceError> for ImportFailure {
    fn from(error: SourceError) -> Self {
        match error {
            SourceError::Offline => Self::Offline,
            SourceError::Refused(reason) => Self::Entry(reason),
        }
    }
}

async fn import_entry<S: EnvironmentSource>(
    source: &S,
    snapshot: &SourceSnapshot,
    target: &ImportTarget<'_>,
    entry: &EnvironmentOperationEntry,
    landed_bytes: &mut u64,
) -> Result<(), ImportFailure> {
    let missing_at_source = || ImportFailure::Entry(format!("Not on {}", snapshot.label));
    match entry.kind {
        EnvironmentEntryKind::Secret => {
            if !snapshot.secrets.contains(&entry.name) {
                return Err(missing_at_source());
            }
            if secret_here(target, &entry.name).map_err(vault_failure)? {
                return Err(ImportFailure::Entry("Already in the Vault".into()));
            }
            let value = source
                .pull_secret(target.project_id, &entry.name)
                .await
                .map_err(ImportFailure::from)?
                .ok_or_else(missing_at_source)?;
            // A value saved while the pull was pending wins over the imported one.
            let stored = target
                .vault
                .set_secret_if_absent(
                    &project_environment_vault_service(target.project_id),
                    &entry.name,
                    &value,
                )
                .map_err(vault_failure)?;
            if stored {
                Ok(())
            } else {
                Err(ImportFailure::Entry("Already in the Vault".into()))
            }
        }
        EnvironmentEntryKind::Mcp | EnvironmentEntryKind::Skill => {
            let held = if entry.kind == EnvironmentEntryKind::Mcp {
                &snapshot.mcps
            } else {
                &snapshot.skills
            };
            if !held.contains(&entry.name) {
                return Err(missing_at_source());
            }
            if target.registrations.has(entry.kind, &entry.name) {
                return Err(ImportFailure::Entry("Already registered here".into()));
            }
            let registration = source
                .pull_registration(target.project_id, entry.kind, &entry.name)
                .await
                .map_err(ImportFailure::from)?
                .ok_or_else(missing_at_source)?;
            // The check above ran before the pull; a registration saved meanwhile wins over the imported one.
            let installed = target
                .registrations
                .install_if_absent(registration)
                .map_err(ImportFailure::Entry)?;
            if installed {
                Ok(())
            } else {
                Err(ImportFailure::Entry("Already registered here".into()))
            }
        }
        EnvironmentEntryKind::File => {
            if !snapshot.files.contains(&entry.name) {
                return Err(missing_at_source());
            }
            if file_here(target, &entry.name) {
                return Err(ImportFailure::Entry("Already present here".into()));
            }
            let first = pull_slice(source, target, &entry.name, 0, missing_at_source).await?;
            let size = first.size_bytes;
            if size > MAX_IMPORT_FILE_BYTES
                || landed_bytes.saturating_add(size) > MAX_IMPORT_TOTAL_BYTES
            {
                return Err(ImportFailure::Entry("Too large to import".into()));
            }
            let step = first.bytes.len() as u64;
            let mut writer =
                FileWriter::create(target.checkout, &entry.name).map_err(ImportFailure::Entry)?;
            writer.append(&first.bytes).map_err(ImportFailure::Entry)?;
            let mut assembled = Sha256::new();
            assembled.update(&first.bytes);
            let mut written = step;
            if written < size && step == 0 {
                return Err(changed());
            }
            // The first slice sets the step; the rest are requested ahead so the link never waits for a round trip.
            let mut slices = futures_util::stream::iter((step..size).step_by(step.max(1) as usize))
                .map(|offset| async move {
                    (
                        offset,
                        pull_slice(source, target, &entry.name, offset, missing_at_source).await,
                    )
                })
                .buffered(SLICES_IN_FLIGHT);
            while let Some((offset, slice)) = slices.next().await {
                let slice = slice?;
                if slice.size_bytes != size
                    || slice.sha256 != first.sha256
                    || slice.bytes.len() as u64 != step.min(size - offset)
                {
                    return Err(changed());
                }
                writer.append(&slice.bytes).map_err(ImportFailure::Entry)?;
                assembled.update(&slice.bytes);
                written += slice.bytes.len() as u64;
            }
            drop(slices);
            // Slices read before and after a same-size replacement agree on size but not content.
            if format!("{:x}", assembled.finalize()) != first.sha256 {
                return Err(changed());
            }
            writer.finish().map_err(ImportFailure::Entry)?;
            *landed_bytes += written;
            Ok(())
        }
    }
}

async fn pull_slice<S: EnvironmentSource>(
    source: &S,
    target: &ImportTarget<'_>,
    path: &str,
    offset: u64,
    missing_at_source: impl FnOnce() -> ImportFailure,
) -> Result<FileSlice, ImportFailure> {
    source
        .pull_file(target.project_id, path, offset)
        .await
        .map_err(ImportFailure::from)?
        .ok_or_else(missing_at_source)
}

fn changed() -> ImportFailure {
    ImportFailure::Entry("The file changed while it was imported".into())
}

fn vault_failure(error: DaemonError) -> ImportFailure {
    if crate::secret::is_chariox_vault_locked_error(&error) {
        ImportFailure::Entry("Vault locked".into())
    } else {
        ImportFailure::Entry("The Vault could not be written".into())
    }
}

/// The names the source can hand over, for its snapshot.
pub(crate) fn source_holdings(
    store: &ProjectEnvironmentStore,
    project_id: &str,
    vault: &dyn CredentialVaultStore,
    registrations: &dyn RegistrationHost,
    checkout: &Path,
    label: String,
) -> Result<SourceSnapshot, DaemonError> {
    let record = store.load(project_id)?;
    let Some(listed) = &record else {
        return Ok(SourceSnapshot {
            label,
            record: None,
            secrets: Vec::new(),
            files: Vec::new(),
            mcps: Vec::new(),
            skills: Vec::new(),
        });
    };
    let held = |kind, entries: &[ExtensionEntry]| -> Vec<String> {
        entries
            .iter()
            .filter(|entry| entry.file_path().is_none() && registrations.has(kind, &entry.name))
            .map(|entry| entry.name.clone())
            .collect()
    };
    let mcps = held(EnvironmentEntryKind::Mcp, &listed.mcps);
    let skills = held(EnvironmentEntryKind::Skill, &listed.skills);
    let service = project_environment_vault_service(project_id);
    let mut secrets = Vec::new();
    for name in &listed.secrets {
        match vault.find_secret(&service, name) {
            Ok(Some(_)) => secrets.push(name.clone()),
            Ok(None) => {}
            // A locked Vault cannot tell, so the target tries the row and the pull says that the Vault is locked.
            Err(error) if crate::secret::is_chariox_vault_locked_error(&error) => {
                secrets.push(name.clone())
            }
            Err(_) => return Err(environment_error("the Vault could not be read")),
        }
    }
    let files = listed
        .files
        .iter()
        .filter(|path| {
            matches!(
                super::file_presence(checkout, path),
                FilePresence::Present { .. }
            )
        })
        .cloned()
        .collect();
    Ok(SourceSnapshot {
        label,
        record,
        secrets,
        files,
        mcps,
        skills,
    })
}

#[cfg(test)]
mod tests {
    use super::super::operations::{Begun, EnvironmentOperations, OperationChanges};
    use super::*;
    use crate::local::{EnvironmentOperationKind, EnvironmentOperationView};
    use crate::mcp::{CharioxMcpRegistry, CharioxMcpServerConfig};
    use crate::skill::CharioxSkillRegistry;
    use std::collections::HashMap;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Debug, Default)]
    struct MemoryVault(Mutex<HashMap<(String, String), String>>);

    impl CredentialVaultStore for MemoryVault {
        fn get_secret(&self, service: &str, key: &str) -> Result<String, DaemonError> {
            self.find_secret(service, key)?
                .ok_or_else(|| environment_error("absent"))
        }
        fn find_secret(&self, service: &str, key: &str) -> Result<Option<String>, DaemonError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(&(service.into(), key.into()))
                .cloned())
        }
        fn set_secret(&self, service: &str, key: &str, value: &str) -> Result<(), DaemonError> {
            self.0
                .lock()
                .unwrap()
                .insert((service.into(), key.into()), value.into());
            Ok(())
        }
        fn delete_secret(&self, service: &str, key: &str) -> Result<(), DaemonError> {
            self.0.lock().unwrap().remove(&(service.into(), key.into()));
            Ok(())
        }
    }

    /// A source that answers from memory, can go offline, and counts what was pulled.
    struct FakeSource {
        snapshot: Result<SourceSnapshot, SourceError>,
        pulls: Mutex<Vec<String>>,
        offline_after_read: bool,
        gate: Option<Arc<tokio::sync::Notify>>,
        /// A secret pull waits for this, so a test can act while the pull is pending.
        secret_gate: Option<Arc<tokio::sync::Notify>>,
        /// The pull of this registration waits for the signal, so a test can act while it is pending.
        registration_gate: Option<(String, Arc<tokio::sync::Notify>)>,
        slice_bytes: usize,
        /// A pull holds this long, so slices requested ahead overlap.
        pull_delay: Duration,
        in_flight: std::sync::atomic::AtomicUsize,
        most_in_flight: std::sync::atomic::AtomicUsize,
        registrations: Vec<(EnvironmentEntryKind, String, Registration)>,
        /// Files come from this checkout, and each is replaced by different bytes of the same size
        /// once its first slice was read.
        replaced_checkout: Option<std::path::PathBuf>,
    }

    /// The registrations of the kernel under test, in memory.
    #[derive(Default)]
    struct MemoryRegistrations {
        held: Mutex<Vec<(EnvironmentEntryKind, String)>>,
        installed: Mutex<Vec<Registration>>,
    }

    impl RegistrationHost for MemoryRegistrations {
        fn has(&self, kind: EnvironmentEntryKind, name: &str) -> bool {
            self.held
                .lock()
                .unwrap()
                .iter()
                .any(|(k, n)| *k == kind && n == name)
        }
        fn read(&self, _: EnvironmentEntryKind, _: &str) -> Result<Option<Registration>, String> {
            Ok(None)
        }
        fn install_if_absent(&self, registration: Registration) -> Result<bool, String> {
            self.installed.lock().unwrap().push(registration);
            Ok(true)
        }
    }

    /// The registrations of the kernel under test on disk, written the way the user's are.
    struct DiskRegistrations {
        mcps: CharioxMcpRegistry,
        skills: CharioxSkillRegistry,
    }

    impl DiskRegistrations {
        fn under(root: &Path) -> Self {
            Self {
                mcps: CharioxMcpRegistry::new(vec![root.join("mcps")]),
                skills: CharioxSkillRegistry::new(vec![root.join("skills")]),
            }
        }
    }

    impl RegistrationHost for DiskRegistrations {
        fn has(&self, kind: EnvironmentEntryKind, name: &str) -> bool {
            match kind {
                EnvironmentEntryKind::Mcp => self.mcps.get(name).unwrap().is_some(),
                EnvironmentEntryKind::Skill => self.skills.get(name).unwrap().is_some(),
                EnvironmentEntryKind::Secret | EnvironmentEntryKind::File => false,
            }
        }
        fn read(&self, _: EnvironmentEntryKind, _: &str) -> Result<Option<Registration>, String> {
            Ok(None)
        }
        fn install_if_absent(&self, registration: Registration) -> Result<bool, String> {
            super::super::registrations::install_if_absent(&self.mcps, &self.skills, registration)
        }
    }

    fn entries_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|read| {
                read.flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// Runs an Import of `names` (all registrations of `kind`) whose pull of `late` is held until `late` was saved
    /// locally by `save`, as another writer on this kernel would.
    async fn import_while_saving(
        rig: &Rig,
        host: &DiskRegistrations,
        kind: EnvironmentEntryKind,
        names: &[&str],
        pulled: Vec<(EnvironmentEntryKind, String, Registration)>,
        save: impl FnOnce(),
    ) -> EnvironmentOperationView {
        let gate = Arc::new(tokio::sync::Notify::new());
        let mut saved = record(1, &[], &[]);
        let entries = names
            .iter()
            .map(|name| ExtensionEntry::registration(*name))
            .collect();
        let held = names.iter().map(|name| name.to_string()).collect();
        if kind == EnvironmentEntryKind::Mcp {
            saved.mcps = entries;
        } else {
            saved.skills = entries;
        }
        let mut from = source(Some(saved), &[], &[]);
        if let Ok(snapshot) = &mut from.snapshot {
            if kind == EnvironmentEntryKind::Mcp {
                snapshot.mcps = held;
            } else {
                snapshot.skills = held;
            }
        }
        from.registrations = pulled;
        from.registration_gate = Some(("late".into(), gate.clone()));
        let running = rig.import_into("pending", &from, host, None);
        let saver = async {
            while from.pulls.lock().unwrap().is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            save();
            gate.notify_one();
        };
        tokio::join!(running, saver).0
    }

    impl EnvironmentSource for FakeSource {
        async fn read(&self, _: &str) -> Result<SourceSnapshot, SourceError> {
            self.snapshot.clone()
        }
        async fn pull_secret(
            &self,
            _: &str,
            name: &str,
        ) -> Result<Option<Zeroizing<String>>, SourceError> {
            self.pulls.lock().unwrap().push(name.into());
            if let Some(gate) = &self.secret_gate {
                gate.notified().await;
            }
            if self.offline_after_read {
                return Err(SourceError::Offline);
            }
            Ok(Some(Zeroizing::new(format!("synthetic-{name}"))))
        }
        async fn pull_registration(
            &self,
            _: &str,
            kind: EnvironmentEntryKind,
            name: &str,
        ) -> Result<Option<Registration>, SourceError> {
            self.pulls.lock().unwrap().push(format!("{kind:?}:{name}"));
            if let Some((gated, gate)) = &self.registration_gate {
                if gated == name {
                    gate.notified().await;
                }
            }
            Ok(self
                .registrations
                .iter()
                .find(|(k, n, _)| *k == kind && n == name)
                .map(|(_, _, registration)| registration.clone()))
        }
        async fn pull_file(
            &self,
            _: &str,
            path: &str,
            offset: u64,
        ) -> Result<Option<FileSlice>, SourceError> {
            self.pulls.lock().unwrap().push(format!("{path}@{offset}"));
            if let Some(checkout) = &self.replaced_checkout {
                let file = checkout.join(path);
                let slice = read_file_slice(&file, offset).unwrap();
                if offset == 0 {
                    let size = std::fs::metadata(&file).unwrap().len() as usize;
                    std::fs::write(&file, vec![b'b'; size]).unwrap();
                }
                return Ok(Some(slice));
            }
            if let Some(gate) = &self.gate {
                gate.notified().await;
            }
            let flying = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.most_in_flight.fetch_max(flying, Ordering::SeqCst);
            tokio::time::sleep(self.pull_delay).await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            let content = format!("content of {path}").into_bytes();
            let end = content.len().min(offset as usize + self.slice_bytes);
            Ok(Some(FileSlice {
                size_bytes: content.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&content)),
                bytes: Zeroizing::new(content[offset as usize..end].to_vec()),
            }))
        }
    }

    fn source(
        record: Option<ProjectEnvironmentRecord>,
        secrets: &[&str],
        files: &[&str],
    ) -> FakeSource {
        FakeSource {
            snapshot: Ok(SourceSnapshot {
                label: "A".into(),
                record,
                secrets: secrets.iter().map(|name| name.to_string()).collect(),
                files: files.iter().map(|name| name.to_string()).collect(),
                mcps: Vec::new(),
                skills: Vec::new(),
            }),
            registrations: Vec::new(),
            pulls: Mutex::default(),
            offline_after_read: false,
            gate: None,
            secret_gate: None,
            registration_gate: None,
            slice_bytes: usize::MAX / 2,
            pull_delay: Duration::ZERO,
            in_flight: Default::default(),
            most_in_flight: Default::default(),
            replaced_checkout: None,
        }
    }

    fn record(revision: u64, secrets: &[&str], files: &[&str]) -> ProjectEnvironmentRecord {
        ProjectEnvironmentRecord {
            schema: super::super::PROJECT_ENVIRONMENT_SCHEMA,
            revision,
            setup_script: "make".into(),
            secrets: secrets.iter().map(|name| name.to_string()).collect(),
            files: files.iter().map(|name| name.to_string()).collect(),
            changed_at_ms: 1,
            ..Default::default()
        }
    }

    struct Rig {
        root: std::path::PathBuf,
        checkout: std::path::PathBuf,
        store: ProjectEnvironmentStore,
        vault: MemoryVault,
        registrations: MemoryRegistrations,
        operations: Arc<EnvironmentOperations>,
        adopted: Mutex<Vec<u64>>,
    }

    impl Rig {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "chariox-env-import-{:032x}",
                rand::random::<u128>()
            ));
            let checkout = root.join("checkout");
            std::fs::create_dir_all(&checkout).unwrap();
            Self {
                store: ProjectEnvironmentStore::new(&root.join("state")),
                operations: EnvironmentOperations::new(
                    &root.join("state"),
                    OperationChanges::new(),
                ),
                vault: MemoryVault::default(),
                registrations: MemoryRegistrations::default(),
                adopted: Mutex::default(),
                checkout,
                root,
            }
        }

        fn begin(&self, id: &str) -> super::super::operations::OperationHandle {
            let view = EnvironmentOperationView {
                operation_id: id.into(),
                project_id: "p".into(),
                checkout_path: self.checkout.to_string_lossy().into_owned(),
                kind: EnvironmentOperationKind::Import,
                state: EnvironmentOperationState::Running,
                started_at_ms: 1,
                ended_at_ms: None,
                exit_code: None,
                reason: None,
                updates: 0,
                output_tail: String::new(),
                entries: Vec::new(),
                source_label: None,
            };
            match self.operations.begin(view).unwrap() {
                Begun::Started(handle) => handle,
                Begun::Existing(_) => panic!("new operation"),
            }
        }

        async fn import<S: EnvironmentSource>(
            &self,
            id: &str,
            source: &S,
            requested: Option<Vec<EnvironmentEntryRef>>,
        ) -> EnvironmentOperationView {
            self.import_into(id, source, &self.registrations, requested)
                .await
        }

        async fn import_into<S: EnvironmentSource>(
            &self,
            id: &str,
            source: &S,
            registrations: &dyn RegistrationHost,
            requested: Option<Vec<EnvironmentEntryRef>>,
        ) -> EnvironmentOperationView {
            let on_record = |record: &ProjectEnvironmentRecord| {
                self.adopted.lock().unwrap().push(record.revision)
            };
            run_import(
                self.begin(id),
                source,
                ImportTarget {
                    project_id: "p",
                    checkout: &self.checkout,
                    store: &self.store,
                    vault: &self.vault,
                    registrations,
                    on_record: &on_record,
                },
                requested,
            )
            .await;
            self.operations.get(id).unwrap().unwrap()
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn states(
        view: &EnvironmentOperationView,
    ) -> Vec<(String, EnvironmentOperationEntryState, Option<String>)> {
        view.entries
            .iter()
            .map(|entry| (entry.name.clone(), entry.state, entry.reason.clone()))
            .collect()
    }

    #[tokio::test]
    async fn update_adopts_the_record_then_brings_each_missing_entry_and_names_the_one_the_source_lacks(
    ) {
        use std::os::unix::fs::PermissionsExt;
        let rig = Rig::new();
        let from = source(
            Some(record(2, &["A_KEY", "B_KEY"], &["config/s.json"])),
            &["A_KEY"],
            &["config/s.json"],
        );
        let view = rig.import("one", &from, None).await;
        assert_eq!(view.source_label.as_deref(), Some("A"));
        assert_eq!(
            states(&view),
            vec![
                (
                    "A_KEY".into(),
                    EnvironmentOperationEntryState::Imported,
                    None
                ),
                (
                    "B_KEY".into(),
                    EnvironmentOperationEntryState::Failed,
                    Some("Not on A".into())
                ),
                (
                    "config/s.json".into(),
                    EnvironmentOperationEntryState::Imported,
                    None
                ),
            ]
        );
        assert_eq!(view.state, EnvironmentOperationState::Failed);
        assert_eq!(rig.store.load("p").unwrap().unwrap().revision, 2);
        assert_eq!(*rig.adopted.lock().unwrap(), vec![2]);
        let service = project_environment_vault_service("p");
        assert_eq!(
            rig.vault.find_secret(&service, "A_KEY").unwrap().as_deref(),
            Some("synthetic-A_KEY")
        );
        assert!(rig.vault.find_secret(&service, "B_KEY").unwrap().is_none());
        let file = rig.checkout.join("config/s.json");
        assert_eq!(std::fs::read(&file).unwrap(), b"content of config/s.json");
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // Nothing the source holds is requested twice, and the record carried no value.
        assert_eq!(
            *from.pulls.lock().unwrap(),
            vec!["A_KEY".to_string(), "config/s.json@0".to_string()]
        );
        assert!(!serde_json::to_string(&view)
            .unwrap()
            .contains("synthetic-A_KEY"));
    }

    #[tokio::test]
    async fn a_file_arrives_in_slices_and_appears_only_when_complete() {
        let rig = Rig::new();
        let mut from = source(Some(record(1, &[], &["big.bin"])), &[], &["big.bin"]);
        from.slice_bytes = 5;
        let view = rig.import("slices", &from, None).await;
        assert_eq!(view.state, EnvironmentOperationState::Succeeded);
        assert_eq!(
            std::fs::read(rig.checkout.join("big.bin")).unwrap(),
            b"content of big.bin"
        );
        assert_eq!(
            *from.pulls.lock().unwrap(),
            vec![
                "big.bin@0".to_string(),
                "big.bin@5".into(),
                "big.bin@10".into(),
                "big.bin@15".into()
            ]
        );
        let leftovers = std::fs::read_dir(&rig.checkout)
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".chariox-import-")
            })
            .count();
        assert_eq!(leftovers, 0);
    }

    #[tokio::test]
    async fn an_mcp_registration_arrives_as_a_row_and_one_held_here_is_left_alone() {
        let rig = Rig::new();
        rig.registrations
            .held
            .lock()
            .unwrap()
            .push((EnvironmentEntryKind::Mcp, "kept".into()));
        let mut saved = record(1, &[], &[]);
        saved.mcps = ["notion", "kept", "gone"]
            .map(ExtensionEntry::registration)
            .to_vec();
        saved.mcps.push(ExtensionEntry::file("github", ".mcp.json"));
        let mut from = source(Some(saved), &[], &[]);
        let config = crate::mcp::CharioxMcpServerConfig::stdio("notion", "npx", vec![]);
        if let Ok(snapshot) = &mut from.snapshot {
            snapshot.mcps = vec!["notion".into(), "kept".into()];
        }
        from.registrations = vec![(
            EnvironmentEntryKind::Mcp,
            "notion".into(),
            Registration::Mcp(config.clone()),
        )];
        let view = rig.import("mcp", &from, None).await;
        assert_eq!(
            states(&view),
            vec![
                (
                    "notion".to_string(),
                    EnvironmentOperationEntryState::Imported,
                    None
                ),
                (
                    "gone".to_string(),
                    EnvironmentOperationEntryState::Failed,
                    Some("Not on A".to_string())
                ),
            ],
            "a file entry is Git's and the registration held here is not asked for"
        );
        assert_eq!(
            *rig.registrations.installed.lock().unwrap(),
            vec![Registration::Mcp(config)]
        );
    }

    #[tokio::test]
    async fn imported_http_mcp_credential_headers_use_portable_project_secrets() {
        use crate::mcp::{
            CharioxMcpCredentialBinding, CharioxMcpServerConfig, CharioxMcpTransportConfig,
        };
        use std::collections::BTreeMap;
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::TcpListener;

        crate::test_support::isolated_env_test!();
        std::env::set_var("HTTP_TEST_TOKEN", "unrelated-ambient-account");
        std::env::set_var("HTTP_TEST_HEADER_X_API_KEY", "unrelated-ambient-header");
        std::env::set_var("HTTP_TEST_FALLBACK", "ambient-fallback");
        let rig = Rig::new();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut config = CharioxMcpServerConfig::streamable_http(
            "http-test",
            format!("http://{}/mcp", listener.local_addr().unwrap()),
        );
        if let CharioxMcpTransportConfig::StreamableHttp {
            bearer_token_credential,
            env_http_headers,
            credential_http_headers,
            ..
        } = &mut config.transport
        {
            *bearer_token_credential = Some("source-vault-handle".into());
            credential_http_headers.insert(
                "X-Api-Key".into(),
                CharioxMcpCredentialBinding {
                    credential: "source-header-handle".into(),
                },
            );
            env_http_headers.insert("X-Fallback".into(), "HTTP_TEST_FALLBACK".into());
        }
        let keys = super::super::registrations::mcp_key_names(&config);
        assert!(
            keys.contains(&"HTTP_TEST_HEADER_X_API_KEY".to_string()),
            "credential header needs a Project Secret"
        );
        let portable = super::super::registrations::portable_mcp(config);
        let mut saved = record(1, &["HTTP_TEST_TOKEN", "HTTP_TEST_HEADER_X_API_KEY"], &[]);
        saved.mcps.push(ExtensionEntry::registration("http-test"));
        let mut from = source(
            Some(saved),
            &["HTTP_TEST_TOKEN", "HTTP_TEST_HEADER_X_API_KEY"],
            &[],
        );
        if let Ok(snapshot) = &mut from.snapshot {
            snapshot.mcps.push("http-test".into());
        }
        from.registrations.push((
            EnvironmentEntryKind::Mcp,
            "http-test".into(),
            Registration::Mcp(portable),
        ));
        assert_eq!(
            rig.import("http-auth", &from, None).await.state,
            EnvironmentOperationState::Succeeded
        );
        let Registration::Mcp(imported) = rig.registrations.installed.lock().unwrap()[0].clone()
        else {
            panic!("imported MCP")
        };
        let service = project_environment_vault_service("p");
        let values = ["HTTP_TEST_TOKEN", "HTTP_TEST_HEADER_X_API_KEY"]
            .into_iter()
            .map(|name| (name.into(), rig.vault.get_secret(&service, name).unwrap()))
            .collect::<BTreeMap<_, _>>();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(&mut stream);
            let mut headers = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
                headers.push_str(&line);
            }
            reader.read_exact(&mut vec![0; length]).unwrap();
            let body = r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            headers
        });
        let response = crate::provider::dispatch_provider_mcp_proxy_request_with_environment(
            "http-import-run",
            "http-import-session",
            &imported,
            serde_json::json!({"jsonrpc":"2.0", "id":1, "method":"tools/call"}),
            &values,
        )
        .unwrap();
        assert_eq!(response["result"]["ok"], true);
        let headers = server.join().unwrap();
        assert!(
            headers.contains("Authorization: Bearer synthetic-HTTP_TEST_TOKEN\r\n"),
            "imported Project bearer was not used"
        );
        assert!(
            headers.contains("X-Api-Key: synthetic-HTTP_TEST_HEADER_X_API_KEY\r\n"),
            "imported Project header was not used"
        );
        assert!(headers.contains("X-Fallback: ambient-fallback\r\n"));
        assert!(!headers.contains("unrelated-ambient"));
    }

    #[tokio::test]
    async fn slices_are_requested_ahead_and_land_in_order() {
        let rig = Rig::new();
        let mut from = source(Some(record(1, &[], &["big.bin"])), &[], &["big.bin"]);
        from.slice_bytes = 3;
        from.pull_delay = Duration::from_millis(20);
        let view = rig.import("ahead", &from, None).await;
        assert_eq!(view.state, EnvironmentOperationState::Succeeded);
        assert_eq!(
            std::fs::read(rig.checkout.join("big.bin")).unwrap(),
            b"content of big.bin"
        );
        let most = from.most_in_flight.load(Ordering::SeqCst);
        assert!(
            (2..=SLICES_IN_FLIGHT).contains(&most),
            "slices in flight at once: {most}"
        );
    }

    #[tokio::test]
    async fn a_file_replaced_by_same_size_content_between_slices_fails_and_is_not_published() {
        let rig = Rig::new();
        let checkout = rig.root.join("source-checkout");
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(
            checkout.join("big.bin"),
            vec![b'a'; 2 * IMPORT_CHUNK_BYTES as usize + 10],
        )
        .unwrap();
        let mut from = source(Some(record(1, &[], &["big.bin"])), &[], &["big.bin"]);
        from.replaced_checkout = Some(checkout);
        let view = rig.import("replaced", &from, None).await;
        assert_eq!(
            from.pulls.lock().unwrap()[..2],
            [
                "big.bin@0".to_string(),
                format!("big.bin@{IMPORT_CHUNK_BYTES}")
            ]
        );
        assert_eq!(
            view.entries[0].reason.as_deref(),
            Some("The file changed while it was imported")
        );
        assert!(!rig.checkout.join("big.bin").exists());
        let leftovers = std::fs::read_dir(&rig.checkout).unwrap().flatten().count();
        assert_eq!(leftovers, 0);
    }

    #[tokio::test]
    async fn a_folder_that_links_out_of_the_checkout_is_not_written_into() {
        let rig = Rig::new();
        let outside = rig.root.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, rig.checkout.join("config")).unwrap();
        let from = source(
            Some(record(1, &[], &["config/s.json"])),
            &[],
            &["config/s.json"],
        );
        let view = rig.import("link", &from, None).await;
        assert_eq!(
            view.entries[0].reason.as_deref(),
            Some("The file would land outside the project")
        );
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn a_file_that_stops_arriving_fails_and_leaves_nothing_behind() {
        let rig = Rig::new();
        let mut from = source(Some(record(1, &[], &["cut.bin"])), &[], &["cut.bin"]);
        from.slice_bytes = 0;
        let view = rig.import("cut", &from, None).await;
        assert_eq!(
            view.entries[0].reason.as_deref(),
            Some("The file changed while it was imported")
        );
        assert!(!rig.checkout.join("cut.bin").exists());
        let leftovers = std::fs::read_dir(&rig.checkout).unwrap().flatten().count();
        assert_eq!(leftovers, 0);
    }

    #[tokio::test]
    async fn an_older_source_record_does_not_replace_this_kernels_newer_one() {
        let rig = Rig::new();
        rig.store.adopt("p", record(5, &["LOCAL"], &[])).unwrap();
        let from = source(Some(record(2, &["OTHER"], &[])), &["OTHER"], &[]);
        let view = rig.import("two", &from, None).await;
        assert_eq!(
            rig.store.load("p").unwrap().unwrap().secrets,
            vec!["LOCAL".to_string()]
        );
        assert!(rig.adopted.lock().unwrap().is_empty());
        assert_eq!(states(&view)[0].0, "LOCAL");
    }

    /// A Vault that holds the value but is locked: every read says so.
    #[derive(Debug)]
    struct LockedVault;

    impl CredentialVaultStore for LockedVault {
        fn get_secret(&self, service: &str, key: &str) -> Result<String, DaemonError> {
            self.find_secret(service, key)?
                .ok_or_else(|| environment_error("absent"))
        }
        fn find_secret(&self, _: &str, _: &str) -> Result<Option<String>, DaemonError> {
            Err(DaemonError::LocalTransport {
                operation: "credential_vault_locked",
                message: "locked".into(),
            })
        }
        fn set_secret(&self, _: &str, _: &str, _: &str) -> Result<(), DaemonError> {
            unreachable!("a snapshot never writes")
        }
        fn delete_secret(&self, _: &str, _: &str) -> Result<(), DaemonError> {
            unreachable!("a snapshot never writes")
        }
    }

    #[test]
    fn a_source_with_a_locked_vault_still_lists_its_secrets_so_the_row_says_the_vault_is_locked() {
        let rig = Rig::new();
        rig.store
            .adopt("p", record(1, &["A_KEY", "B_KEY"], &[]))
            .unwrap();
        let held = source_holdings(
            &rig.store,
            "p",
            &LockedVault,
            &rig.registrations,
            &rig.checkout,
            "A".into(),
        )
        .unwrap();
        assert_eq!(
            held.secrets,
            ["A_KEY", "B_KEY"],
            "a locked Vault cannot tell, so the target tries each row and the pull names the lock"
        );
    }

    async fn update_with_a_locked_vault(
        rig: &Rig,
        id: &str,
        from: &FakeSource,
    ) -> EnvironmentOperationView {
        let on_record =
            |record: &ProjectEnvironmentRecord| rig.adopted.lock().unwrap().push(record.revision);
        run_import(
            rig.begin(id),
            from,
            ImportTarget {
                project_id: "p",
                checkout: &rig.checkout,
                store: &rig.store,
                vault: &LockedVault,
                registrations: &rig.registrations,
                on_record: &on_record,
            },
            None,
        )
        .await;
        rig.operations.get(id).unwrap().unwrap()
    }

    #[tokio::test]
    async fn an_update_that_resolves_to_a_secret_is_refused_whole_while_the_vault_is_locked() {
        let rig = Rig::new();
        let from = source(
            Some(record(2, &["A_KEY"], &["config/s.json"])),
            &["A_KEY"],
            &["config/s.json"],
        );
        let view = update_with_a_locked_vault(&rig, "locked", &from).await;
        assert_eq!(view.state, EnvironmentOperationState::Failed);
        assert!(
            rig.store.load("p").unwrap().is_none(),
            "the newer record is not adopted"
        );
        assert!(rig.adopted.lock().unwrap().is_empty());
        assert!(
            !rig.checkout.join("config/s.json").exists(),
            "no File lands beside the refused Secret"
        );
        assert!(from.pulls.lock().unwrap().is_empty());
        assert!(
            view.reason
                .as_deref()
                .is_some_and(|reason| reason.contains("Vault is locked")),
            "{:?}",
            view.reason
        );
    }

    #[tokio::test]
    async fn an_update_without_a_secret_does_not_need_the_vault() {
        let rig = Rig::new();
        let from = source(
            Some(record(2, &[], &["config/s.json"])),
            &[],
            &["config/s.json"],
        );
        let view = update_with_a_locked_vault(&rig, "files-only", &from).await;
        assert_eq!(view.state, EnvironmentOperationState::Succeeded, "{view:?}");
        assert_eq!(rig.store.load("p").unwrap().unwrap().revision, 2);
        assert!(rig.checkout.join("config/s.json").exists());
    }

    #[tokio::test]
    async fn an_offline_source_fails_every_row_after_one_attempt() {
        let rig = Rig::new();
        let mut from = source(
            Some(record(1, &["A_KEY", "B_KEY"], &["x"])),
            &["A_KEY", "B_KEY"],
            &["x"],
        );
        from.offline_after_read = true;
        let view = rig.import("three", &from, None).await;
        assert!(view
            .entries
            .iter()
            .all(|entry| entry.reason.as_deref() == Some(SOURCE_OFFLINE)));
        assert_eq!(from.pulls.lock().unwrap().len(), 1);
        let mut gone = source(None, &[], &[]);
        gone.snapshot = Err(SourceError::Offline);
        let view = rig.import("four", &gone, None).await;
        assert_eq!(view.state, EnvironmentOperationState::Failed);
        assert_eq!(view.reason.as_deref(), Some(SOURCE_OFFLINE));
    }

    #[tokio::test]
    async fn a_named_entry_outside_the_record_or_already_here_fails_on_its_own_row() {
        let rig = Rig::new();
        rig.store
            .adopt("p", record(1, &["HAVE", "NEW"], &[".env"]))
            .unwrap();
        rig.vault
            .set_secret(&project_environment_vault_service("p"), "HAVE", "kept")
            .unwrap();
        std::fs::write(rig.checkout.join(".env"), b"kept").unwrap();
        let from = source(
            Some(record(1, &["HAVE", "NEW"], &[".env"])),
            &["HAVE", "NEW"],
            &[".env"],
        );
        let asked = vec![
            EnvironmentEntryRef {
                kind: EnvironmentEntryKind::Secret,
                name: "STRANGER".into(),
            },
            EnvironmentEntryRef {
                kind: EnvironmentEntryKind::Secret,
                name: "HAVE".into(),
            },
            EnvironmentEntryRef {
                kind: EnvironmentEntryKind::File,
                name: ".env".into(),
            },
            EnvironmentEntryRef {
                kind: EnvironmentEntryKind::Secret,
                name: "NEW".into(),
            },
        ];
        let view = rig.import("five", &from, Some(asked)).await;
        let reasons: Vec<_> = states(&view)
            .into_iter()
            .map(|(_, state, reason)| (state, reason))
            .collect();
        assert_eq!(
            reasons,
            vec![
                (
                    EnvironmentOperationEntryState::Failed,
                    Some("Not in this Environment".into())
                ),
                (
                    EnvironmentOperationEntryState::Failed,
                    Some("Already in the Vault".into())
                ),
                (
                    EnvironmentOperationEntryState::Failed,
                    Some("Already present here".into())
                ),
                (EnvironmentOperationEntryState::Imported, None),
            ]
        );
        assert_eq!(std::fs::read(rig.checkout.join(".env")).unwrap(), b"kept");
    }

    #[tokio::test]
    async fn stop_keeps_what_already_landed_and_ends_the_run() {
        let rig = Rig::new();
        let gate = Arc::new(tokio::sync::Notify::new());
        let mut from = source(
            Some(record(1, &["A_KEY"], &["slow"])),
            &["A_KEY"],
            &["slow"],
        );
        from.gate = Some(gate);
        let running = rig.import("six", &from, None);
        let stopper = async {
            loop {
                if rig.operations.get("six").unwrap().is_some_and(|view| {
                    view.entries.iter().any(|entry| {
                        entry.state == EnvironmentOperationEntryState::Importing
                            && entry.name == "slow"
                    })
                }) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            rig.operations.cancel("six").unwrap();
        };
        let (view, ()) = tokio::join!(running, stopper);
        assert_eq!(view.state, EnvironmentOperationState::Cancelled);
        let service = project_environment_vault_service("p");
        assert!(rig.vault.find_secret(&service, "A_KEY").unwrap().is_some());
        assert!(!rig.checkout.join("slow").exists());
    }

    #[tokio::test]
    async fn a_secret_saved_while_its_import_is_pending_is_kept() {
        let rig = Rig::new();
        let gate = Arc::new(tokio::sync::Notify::new());
        let mut from = source(Some(record(1, &["LATE"], &[])), &["LATE"], &[]);
        from.secret_gate = Some(gate.clone());
        let service = project_environment_vault_service("p");
        let running = rig.import("seven", &from, None);
        let saver = async {
            while from.pulls.lock().unwrap().is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            rig.vault
                .set_secret(&service, "LATE", "typed-locally")
                .unwrap();
            gate.notify_one();
        };
        let (view, ()) = tokio::join!(running, saver);
        assert_eq!(
            view.entries[0].state,
            EnvironmentOperationEntryState::Failed
        );
        assert_eq!(
            view.entries[0].reason.as_deref(),
            Some("Already in the Vault")
        );
        assert_eq!(
            rig.vault.find_secret(&service, "LATE").unwrap().as_deref(),
            Some("typed-locally")
        );
    }

    #[tokio::test]
    async fn an_mcp_saved_while_its_import_is_pending_is_kept_byte_for_byte() {
        let rig = Rig::new();
        let host = DiskRegistrations::under(&rig.root);
        let pulled = |name: &str| {
            Registration::Mcp(CharioxMcpServerConfig::stdio(name, "from-source", vec![]))
        };
        let local = CharioxMcpServerConfig::stdio("late", "typed-locally", vec!["--keep".into()]);
        let view = import_while_saving(
            &rig,
            &host,
            EnvironmentEntryKind::Mcp,
            &["late", "free", "broken"],
            vec![
                (EnvironmentEntryKind::Mcp, "late".into(), pulled("late")),
                (EnvironmentEntryKind::Mcp, "free".into(), pulled("free")),
                (
                    EnvironmentEntryKind::Mcp,
                    "broken".into(),
                    Registration::Mcp(CharioxMcpServerConfig::stdio("broken", " ", vec![])),
                ),
            ],
            || {
                host.mcps.install(&local).unwrap();
            },
        )
        .await;
        assert_eq!(
            view.entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.state))
                .collect::<Vec<_>>(),
            vec![
                ("late", EnvironmentOperationEntryState::Failed),
                ("free", EnvironmentOperationEntryState::Imported),
                ("broken", EnvironmentOperationEntryState::Failed),
            ]
        );
        assert_eq!(
            view.entries[0].reason.as_deref(),
            Some("Already registered here")
        );
        let root = rig.root.join("mcps");
        assert_eq!(
            std::fs::read_to_string(root.join("late.json")).unwrap(),
            format!("{}\n", serde_json::to_string_pretty(&local).unwrap())
        );
        assert_eq!(
            host.mcps.get("free").unwrap().unwrap().name,
            "free",
            "a name still absent is imported"
        );
        assert_eq!(
            entries_in(&root),
            vec!["free.json".to_string(), "late.json".into()],
            "the refused and the failed entries leave no file behind"
        );
    }

    #[tokio::test]
    async fn a_skill_saved_while_its_import_is_pending_is_kept_byte_for_byte() {
        let rig = Rig::new();
        let host = DiskRegistrations::under(&rig.root);
        let origin = CharioxSkillRegistry::new(vec![rig.root.join("origin")]);
        let write_skill = |dir: &Path, name: &str, body: &str| {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(
                dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {body}\n---\n{body}\n"),
            )
            .unwrap();
        };
        let staging = rig.root.join("staging");
        let mut pulled = Vec::new();
        for name in ["late", "free", "broken"] {
            write_skill(&staging.join(name), name, "from the source");
            origin.upsert_from_path(&staging.join(name)).unwrap();
            let mut package = origin.package(name).unwrap().unwrap();
            if name == "broken" {
                package.files[0].sha256 = "0".repeat(64);
            }
            pulled.push((
                EnvironmentEntryKind::Skill,
                name.to_string(),
                Registration::Skill(package),
            ));
        }
        let local_dir = rig.root.join("skills").join("late");
        let view = import_while_saving(
            &rig,
            &host,
            EnvironmentEntryKind::Skill,
            &["late", "free", "broken"],
            pulled,
            || {
                write_skill(&local_dir, "late", "typed locally");
                std::fs::write(local_dir.join("notes.txt"), "mine").unwrap();
            },
        )
        .await;
        assert_eq!(
            view.entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.state))
                .collect::<Vec<_>>(),
            vec![
                ("late", EnvironmentOperationEntryState::Failed),
                ("free", EnvironmentOperationEntryState::Imported),
                ("broken", EnvironmentOperationEntryState::Failed),
            ]
        );
        assert_eq!(
            view.entries[0].reason.as_deref(),
            Some("Already registered here")
        );
        assert_eq!(
            std::fs::read_to_string(local_dir.join("SKILL.md")).unwrap(),
            "---\nname: late\ndescription: typed locally\n---\ntyped locally\n"
        );
        assert_eq!(
            std::fs::read_to_string(local_dir.join("notes.txt")).unwrap(),
            "mine"
        );
        assert!(host.skills.get("free").unwrap().is_some());
        assert_eq!(
            entries_in(&rig.root.join("skills")),
            vec!["free".to_string(), "late".into()],
            "the refused and the failed entries leave no directory behind, not even a backup"
        );
    }
}
