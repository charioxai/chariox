//! MP-08 / MP-10 / MP-11: one existing durable journal owns the home catalogs.
//! The JSON file is a recoverable ownership pin, not another state database.
use super::{catalog::failure, storage::SkillCatalogStorage};
use crate::error::DaemonError;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    fs::OpenOptions,
    io::Read,
    path::{Path, PathBuf},
};

const KIND: &str = "skills.v2.home";
const SUBJECT: &str = "skills-v2";
const FILE: &str = "home-journal.json";
const MAX_BINDING_BYTES: u64 = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HomeJournal {
    schema: u32,
    home: PathBuf,
    journal: PathBuf,
    id: String,
}

fn journal_pin(storage: &SkillCatalogStorage) -> Result<Option<HomeJournal>, DaemonError> {
    let events = storage
        .journal()
        .load_subject_events_by_kind(SUBJECT, KIND, 1)?;
    events
        .last()
        .map(|event| {
            let binding: HomeJournal =
                serde_json::from_value(event.payload.clone()).map_err(|_| {
                    failure(
                        "invalid Skills home journal ownership; recover the owning kernel state",
                    )
                })?;
            if binding.schema != 2
                || binding.home != storage.home()
                || binding.journal
                    != fs::canonicalize(storage.journal().path())
                        .map_err(|_| failure("cannot resolve the Skills home journal"))?
                || binding.id.is_empty()
            {
                return Err(failure(
                    "this journal belongs to a different Skills home; use the owning home kernel",
                ));
            }
            Ok(binding)
        })
        .transpose()
}

fn file_pin(path: &Path) -> Result<Option<HomeJournal>, DaemonError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(failure(
                "cannot read Skills home journal ownership; use the owning home kernel",
            ))
        }
    };
    let metadata = file
        .metadata()
        .map_err(|_| failure("cannot inspect Skills home journal ownership"))?;
    if !metadata.is_file() || metadata.len() > MAX_BINDING_BYTES {
        return Err(failure("invalid Skills home journal ownership file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(failure(
                "Skills home journal ownership file is not owned by this user",
            ));
        }
    }
    let mut bytes = Vec::new();
    file.take(MAX_BINDING_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| failure("cannot read Skills home journal ownership"))?;
    if bytes.len() as u64 > MAX_BINDING_BYTES {
        return Err(failure("Skills home journal ownership file is too large"));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| failure("invalid Skills home journal ownership; reopen the owning kernel"))
}

/// Caller holds the ordinary cross-process catalog lock. A second named daemon
/// must route catalog work to the owner or report unavailability; it may not
/// select a new authority by overwriting the shared JSON projections.
pub(super) fn claim(storage: &SkillCatalogStorage) -> Result<(), DaemonError> {
    let directory = storage.directory(Path::new("state/skills"))?;
    let path = directory.join(FILE);
    let durable = journal_pin(storage)?;
    let existing = file_pin(&path)?;
    if let Some(existing) = existing {
        if durable.as_ref() != Some(&existing) {
            return Err(failure("Skills home catalogs belong to another kernel journal; connect to the owning home kernel"));
        }
        return Ok(());
    }
    let binding = match durable {
        Some(binding) => binding,
        None => {
            // An absent pointer is not permission to replace a catalog that was
            // projected from another journal. Recovery may only attest its own
            // existing durable catalog, including pre-pin foundation snapshots.
            let has_projection = match fs::symlink_metadata(directory.join("catalog.json")) {
                Ok(_) => true,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(_) => return Err(failure("cannot inspect existing Skills catalog ownership")),
            };
            if has_projection
                && storage
                    .journal()
                    .load_subject_events_by_kind(SUBJECT, "skills.v2.catalog", 1)?
                    .is_empty()
            {
                return Err(failure("Skills home journal ownership is missing; recover the owning kernel instead of creating an empty catalog"));
            }
            let binding = HomeJournal {
                schema: 2,
                home: storage.home().into(),
                journal: fs::canonicalize(storage.journal().path())
                    .map_err(|_| failure("cannot resolve the Skills home journal"))?,
                id: format!("home-{:032x}", rand::random::<u128>()),
            };
            storage.journal().append_event(
                KIND,
                Some(SUBJECT.into()),
                serde_json::to_value(&binding)
                    .map_err(|_| failure("cannot encode Skills home journal ownership"))?,
            )?;
            binding
        }
    };
    crate::config::write_private_file(
        &path,
        &serde_json::to_vec(&binding)
            .map_err(|_| failure("cannot encode Skills home journal ownership"))?,
    )
    .map_err(|_| failure("cannot publish Skills home journal ownership; reopen the owning kernel"))
}

pub(super) fn verify(storage: &SkillCatalogStorage) -> Result<(), DaemonError> {
    // Do not recreate a missing marker during an ordinary read/write. Recovery
    // takes the catalog lock through new(); a stale handle stays fail-closed.
    let path = storage.directory(Path::new("state/skills"))?.join(FILE);
    let expected = journal_pin(storage)?.ok_or_else(|| {
        failure("Skills home journal ownership is missing; reopen the owning kernel")
    })?;
    if file_pin(&path)?.as_ref() != Some(&expected) {
        return Err(failure(
            "Skills home journal ownership changed; reopen the owning home kernel",
        ));
    }
    Ok(())
}

/// MP-08 / MP-10 / MP-11: a named daemon must not apply its old names to
/// another journal's already upgraded global packages or workflow artifacts.
/// The owner is read only for this bounded cleanup certificate, never sessions.
pub(super) fn completed_foreign_reset(
    storage: &SkillCatalogStorage,
) -> Result<Option<std::collections::BTreeSet<String>>, DaemonError> {
    let directory = storage.directory(Path::new("state/skills"))?;
    let Some(binding) = file_pin(&directory.join(FILE))? else {
        return Ok(None);
    };
    let journal = fs::canonicalize(storage.journal().path())
        .map_err(|_| failure("cannot resolve the Skills home journal"))?;
    if binding.journal == journal {
        return Ok(None);
    }
    if binding.schema != 2
        || binding.home != storage.home()
        || binding.id.is_empty()
        || !binding.journal.is_absolute()
        || fs::canonicalize(&binding.journal).ok().as_ref() != Some(&binding.journal)
    {
        return Err(failure(
            "invalid Skills home ownership; recover the owning kernel",
        ));
    }
    let connection = rusqlite::Connection::open_with_flags(
        &binding.journal,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|_| {
        failure("the Skills home owner is unavailable; restore its journal before retrying startup")
    })?;
    use rusqlite::OptionalExtension;
    let encoded:Option<String>=connection.query_row(
        "SELECT payload_json FROM durable_state_events WHERE kind=?1 AND subject_id=?2 ORDER BY sequence DESC LIMIT 1",
        [KIND,SUBJECT],|row|row.get(0)).optional().map_err(|_|failure("cannot verify the Skills home owner"))?;
    let pinned: HomeJournal = encoded
        .and_then(|encoded| serde_json::from_str(&encoded).ok())
        .ok_or_else(|| failure("cannot verify the Skills home owner"))?;
    if pinned != binding {
        return Err(failure(
            "Skills home ownership changed; recover the owning kernel",
        ));
    }
    let complete:i64=connection.query_row(
        "SELECT count(*) FROM durable_state_metadata WHERE owner_id='skills-v2' AND ((metadata_key='writer_epoch' AND metadata_value='2') OR (metadata_key='executable_reference_reset' AND metadata_value='complete') OR metadata_key='retired_names')",
        [],|row|row.get(0)).map_err(|_|failure("cannot verify the Skills home reset"))?;
    if complete != 3 {
        return Err(failure(
            "the owning kernel must finish Skills cleanup before this kernel can start",
        ));
    }
    crate::durable_state::skill_reset::retired_skill_names(&connection)
        .map(Some)
        .map_err(|_| failure("cannot verify retired names from the Skills home owner"))
}
