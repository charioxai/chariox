//! MP-08 / MP-10 / MP-11: regenerate only owned artifacts through the normal publisher.
use super::*;
use std::io::Read;

fn refused() -> crate::DaemonError {
    crate::DaemonError::LocalTransport {
        operation: "workflow_code.skill_reset",
        message:
            "owned workflow artifact could not be retired; republish it before retrying startup"
                .into(),
    }
}
fn owned(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.uid() == unsafe { libc::geteuid() }
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        true
    }
}
fn unchanged(path: &Path, previous: &fs::Metadata) -> Result<(), crate::DaemonError> {
    let current = fs::symlink_metadata(path).map_err(|_| refused())?;
    if current.file_type().is_symlink()
        || !current.is_file()
        || !owned(&current)
        || current.len() != previous.len()
        || current.modified().ok() != previous.modified().ok()
    {
        return Err(refused());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if current.dev() != previous.dev() || current.ino() != previous.ino() {
            return Err(refused());
        }
    }
    Ok(())
}
fn for_owned_artifacts(
    roots: &[PathBuf],
    mut action: impl FnMut(&Path, fs::Metadata, Value) -> Result<(), crate::DaemonError>,
) -> Result<(), crate::DaemonError> {
    let mut seen = BTreeSet::new();
    for root in roots {
        if !root.is_absolute() {
            return Err(refused());
        }
        let metadata = match fs::symlink_metadata(root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(refused()),
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() || !owned(&metadata) {
            return Err(refused());
        }
        let root = fs::canonicalize(root).map_err(|_| refused())?;
        if !seen.insert(root.clone()) {
            continue;
        }
        for entry in fs::read_dir(&root).map_err(|_| refused())? {
            let path = entry.map_err(|_| refused())?.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|name| name.to_str()) else {
                return Err(refused());
            };
            // Other files are not discoverable registry artifacts.
            if validate_registry_name(name, "workflow-code artifact name").is_err() {
                continue;
            }
            let mut options = fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
            }
            let mut file = options.open(&path).map_err(|_| refused())?;
            let metadata = file.metadata().map_err(|_| refused())?;
            if !metadata.is_file() || !owned(&metadata) {
                return Err(refused());
            }
            let mut encoded = String::new();
            file.read_to_string(&mut encoded).map_err(|_| refused())?;
            unchanged(&path, &metadata)?;
            let value: Value = serde_json::from_str(&encoded).map_err(|_| refused())?;
            if value.get("name").and_then(Value::as_str) != Some(name)
                || value.get("definition").is_none()
            {
                return Err(refused());
            }
            action(&path, metadata, value)?;
        }
    }
    Ok(())
}

pub(crate) fn owned_workflow_skill_names(
    roots: &[PathBuf],
) -> Result<BTreeSet<String>, crate::DaemonError> {
    let mut names = BTreeSet::new();
    for_owned_artifacts(roots, |_, _, value| {
        crate::durable_state::skill_reset::collect_retired_skill_names(
            &value["definition"],
            &mut names,
        );
        Ok(())
    })?;
    Ok(names)
}

pub(crate) fn retire_owned_workflow_skill_artifacts(
    roots: &[PathBuf],
    names: &BTreeSet<String>,
    limits: &WorkflowCodeLimitsConfig,
) -> Result<(), crate::DaemonError> {
    for_owned_artifacts(roots, |path, metadata, mut value| {
        let before = value["definition"].clone();
        let mut local_names = names.clone();
        crate::durable_state::skill_reset::collect_retired_skill_names(&before, &mut local_names);
        crate::durable_state::skill_reset::retire_workflow_skill_references(
            &mut value["definition"],
            &local_names,
        );
        if value["definition"] == before {
            return Ok(());
        }
        let mut stored: StoredWorkflowCodeArtifact =
            serde_json::from_value(value).map_err(|_| refused())?;
        let export = source_export::export_workflow_code_source_from_definition(
            &stored.name,
            &stored.definition,
            WorkflowCodeSourceExportFormat::Inline,
        )?;
        stored.source = export.source;
        stored.source_sha256 = export.source_sha256;
        stored.language = export.language;
        stored.validation = stored.definition.validate_with_limits(limits);
        stored.updated_at_ms = crate::session::unix_epoch_ms();
        let actor = WorkflowCodeArtifactActor::new("skills-v2-reset", None);
        stored.provenance.updated_by = actor.clone();
        stored.history.push(WorkflowCodeArtifactHistoryEntry {
            action: WorkflowCodeArtifactHistoryAction::Updated,
            at_ms: stored.updated_at_ms,
            actor,
            source_sha256: stored.source_sha256.clone(),
            validation_ok: Some(stored.validation.ok),
            workflow_id: None,
            warnings: vec![],
        });
        if stored.history.len() > WORKFLOW_CODE_ARTIFACT_HISTORY_LIMIT {
            stored
                .history
                .drain(0..stored.history.len() - WORKFLOW_CODE_ARTIFACT_HISTORY_LIMIT);
        }
        unchanged(path, &metadata)?;
        artifact_registry::write_stored_artifact(path, &stored)
    })
}
