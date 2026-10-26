use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};
#[cfg(unix)]
use sha2::{Digest, Sha256};

use super::{
    digest, profile_root, NativeCheckpointBinding, NativeCheckpointFailure as Failure,
    ProviderNativeCheckpoint,
};
use crate::provider::{CodexClient, ProviderAuthStatus, ProviderResumeState, RuntimeProviderRun};

#[derive(Deserialize)]
struct MetadataResponse {
    thread: ThreadMetadata,
}
#[derive(Deserialize)]
struct ThreadMetadata {
    id: String,
    path: Option<PathBuf>,
    status: Value,
}

pub(super) fn supported_version(version: &str) -> Result<(), Failure> {
    if version != "codex-cli 0.159.3" {
        return Err(Failure::UnsupportedVersion);
    }
    Ok(())
}

pub(super) fn valid_relative_path(path: &str, id: &str) -> bool {
    let parts = path.split('/').collect::<Vec<_>>();
    let uuid = id.len() == 36
        && id.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        });
    uuid && parts.len() == 5
        && parts[0] == "sessions"
        && parts[1].len() == 4
        && parts[2].len() == 2
        && parts[3].len() == 2
        && parts[1..4]
            .iter()
            .all(|part| part.bytes().all(|b| b.is_ascii_digit()))
        && parts[4].starts_with("rollout-")
        && parts[4].ends_with(&format!("-{id}.jsonl"))
        && parts[4]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

fn auth(client: &CodexClient, profile: &str) -> Result<ProviderAuthStatus, Failure> {
    let auth = client
        .auth_status(profile)
        .map_err(|_| Failure::AccountUnavailable)?;
    if auth.auth_state != "authenticated"
        || auth
            .identity_summary
            .as_deref()
            .is_none_or(|id| id.trim().is_empty())
    {
        return Err(Failure::AccountUnavailable);
    }
    supported_version(
        auth.detected_version
            .as_deref()
            .ok_or(Failure::UnsupportedVersion)?,
    )?;
    Ok(auth)
}

pub(super) fn account_digest(auth: &ProviderAuthStatus) -> Result<String, Failure> {
    let identity = auth
        .identity_summary
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(Failure::AccountUnavailable)?;
    Ok(digest(
        format!("codex:authenticated:{}", identity.to_ascii_lowercase()).as_bytes(),
    ))
}

fn metadata(client: &CodexClient, id: &str) -> Result<ThreadMetadata, crate::DaemonError> {
    let mut socket = client.connect_initialized()?;
    let response: MetadataResponse = client.send_request(
        &mut socket,
        &mut 1,
        "thread/read",
        json!({"threadId":id,"includeTurns":false}),
    )?;
    Ok(response.thread)
}

fn idle(metadata: &ThreadMetadata, id: &str) -> Result<(), Failure> {
    if metadata.id != id {
        return Err(Failure::SessionChanged);
    }
    match metadata.status.get("type").and_then(Value::as_str) {
        Some("idle" | "notLoaded") => Ok(()),
        _ => Err(Failure::NotQuiescent),
    }
}

fn model(model: &str) -> &str {
    model.strip_prefix("codex/").unwrap_or(model)
}
fn variant(value: Option<&str>) -> Option<&str> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "default")
}

#[cfg(unix)]
pub(super) fn export(run: &RuntimeProviderRun) -> Result<ProviderNativeCheckpoint, Failure> {
    let id = run
        .provider_session_id()
        .or_else(|| run.resume_state().codex_thread_id())
        .ok_or(Failure::MissingSession)?;
    // The conversation process has stopped. Read its durable metadata through
    // the ordinary profile-scoped official utility server, without resuming it.
    let root = profile_root(run.pty_env())?;
    let endpoint = crate::provider::ensure_codex_account_endpoint(
        run.owner_user_id(),
        run.account_profile(),
        std::collections::BTreeMap::from([("CODEX_HOME".to_string(), root.display().to_string())]),
    )
    .map_err(|_| Failure::AccountUnavailable)?;
    let client = CodexClient::new(run.id(), endpoint).map_err(|_| Failure::ResumeFailed)?;
    let auth = auth(&client, run.account_profile())?;
    let metadata = metadata(&client, id).map_err(|_| Failure::MissingSession)?;
    idle(&metadata, id)?;
    let root = profile_root(run.pty_env())?;
    let path = metadata.path.as_deref().ok_or(Failure::MissingSession)?;
    let relative_path = path
        .strip_prefix(root)
        .ok()
        .and_then(Path::to_str)
        .ok_or(Failure::InvalidArtifact)?;
    if !valid_relative_path(relative_path, id) {
        return Err(Failure::InvalidArtifact);
    }
    let contents = super::files::Directory::open(root)?.read(relative_path)?;
    let checkpoint = ProviderNativeCheckpoint {
        format: "codex-jsonl-v1".to_string(),
        binding: NativeCheckpointBinding {
            provider: "codex".to_string(),
            session_id: id.to_string(),
            account_sha256: account_digest(&auth)?,
            model: model(run.model()).to_string(),
            variant: variant(run.variant()).map(str::to_string),
            cli_version: auth.detected_version.ok_or(Failure::UnsupportedVersion)?,
            source_os: std::env::consts::OS.to_string(),
            artifact_sha256: digest(&contents),
        },
        relative_path: relative_path.to_string(),
        contents,
    };
    checkpoint.validate()?;
    Ok(checkpoint)
}

#[cfg(unix)]
pub(super) fn import(
    checkpoint: &ProviderNativeCheckpoint,
    run: &RuntimeProviderRun,
) -> Result<ProviderResumeState, Failure> {
    import_checkpoint(checkpoint, run, false)
}

#[cfg(unix)]
pub(super) fn import_for_move(
    checkpoint: &ProviderNativeCheckpoint,
    run: &RuntimeProviderRun,
) -> Result<ProviderResumeState, Failure> {
    import_checkpoint(checkpoint, run, true)
}

#[cfg(unix)]
fn import_checkpoint(
    checkpoint: &ProviderNativeCheckpoint,
    run: &RuntimeProviderRun,
    allow_extension: bool,
) -> Result<ProviderResumeState, Failure> {
    if run.provider() != "codex"
        || model(run.model()) != checkpoint.binding.model
        || variant(run.variant()) != checkpoint.binding.variant.as_deref()
    {
        return Err(Failure::ConfigurationMismatch);
    }
    let root = profile_root(run.pty_env())?;
    let cwd = run
        .working_directory()
        .map(PathBuf::as_path)
        .filter(|path| path.is_absolute() && path.is_dir())
        .ok_or(Failure::ConfigurationMismatch)?;
    let client = CodexClient::new(
        run.id(),
        run.structured_endpoint().ok_or(Failure::ResumeFailed)?,
    )
    .map_err(|_| Failure::ResumeFailed)?;
    let auth = auth(&client, run.account_profile())?;
    if account_digest(&auth)? != checkpoint.binding.account_sha256 {
        return Err(Failure::AccountMismatch);
    }
    let directory = super::files::Directory::open(root)?;
    let id = checkpoint.provider_session_id();
    let already_published = match metadata(&client, id) {
        Ok(existing) => {
            idle(&existing, id)?;
            let relative = existing
                .path
                .as_deref()
                .and_then(|p| p.strip_prefix(root).ok())
                .and_then(Path::to_str)
                .ok_or(Failure::TargetConflict)?;
            prepare_existing_artifact(&directory, relative, checkpoint, allow_extension)?;
            true
        }
        Err(crate::DaemonError::ProviderProtocol {
            operation: "thread/read",
            message,
            ..
        }) if message == format!("thread not loaded: {id}")
            || message == format!("no rollout found for thread id {id}") =>
        {
            false
        }
        Err(_) => return Err(Failure::ResumeFailed),
    };
    if !already_published {
        match directory.publish(&checkpoint.relative_path, &checkpoint.contents) {
            Err(Failure::TargetConflict) if allow_extension => {
                prepare_existing_artifact(&directory, &checkpoint.relative_path, checkpoint, true)?;
            }
            result => result?,
        }
    }
    // Check native continuity before normal startup installs this run's tools
    // and interaction context. Process-level MCP entries must not run in a probe.
    let client = client
        .with_provider_config_overrides(run.provider_config_overrides())
        .with_read_only_discovery_permissions();
    let mut socket = client
        .connect_initialized()
        .map_err(|error| resume_probe_failure(run, "connect", &error))?;
    let resumed = client
        .thread_resume(
            &mut socket,
            &mut 1,
            id,
            Some(&cwd.to_string_lossy()),
            Some(model(run.model())),
            run.write_access_mode(),
            run.execution_mode(),
            run.permission_level(),
            None,
            &mut Vec::new(),
        )
        .map_err(|error| resume_probe_failure(run, "thread/resume", &error))?;
    if resumed.thread.id != id {
        return Err(Failure::SessionChanged);
    }
    if resumed.model != checkpoint.binding.model || resumed.cwd.as_deref() != Some(cwd) {
        return Err(Failure::ConfigurationMismatch);
    }
    Ok(
        ProviderResumeState::from_codex_thread_id(id).requiring_native_session(
            "codex",
            id,
            Some(checkpoint.binding.clone()),
        ),
    )
}

#[cfg(unix)]
fn prepare_existing_artifact(
    directory: &super::files::Directory,
    relative: &str,
    checkpoint: &ProviderNativeCheckpoint,
    allow_extension: bool,
) -> Result<(), Failure> {
    match verify_required_artifact(directory, relative, &checkpoint.binding) {
        Err(Failure::TargetConflict) if allow_extension => {
            // A returned thread extends the stopped home thread. Preserve
            // every home byte and refuse any divergent native history.
            directory.extend(relative, &checkpoint.contents)?;
            verify_required_artifact(directory, relative, &checkpoint.binding)
        }
        result => result,
    }
}

fn resume_probe_failure(
    run: &RuntimeProviderRun,
    stage: &str,
    error: &crate::DaemonError,
) -> Failure {
    let operation = match error {
        crate::DaemonError::ProviderProtocol { operation, .. } => *operation,
        _ => "native_checkpoint",
    };
    let failure_terms = match error {
        crate::DaemonError::ProviderProtocol { message, .. } => {
            let message = message.to_ascii_lowercase();
            [
                "missing field",
                "invalid type",
                "unknown variant",
                "expected",
                "mcp",
                "config",
                "model",
                "thread",
                "rollout",
                "session",
                "not found",
                "already",
                "permission",
                "sandbox",
                "approval",
                "runtime",
                "cwd",
                "path",
                "unsupported",
                "invalid",
                "disabled",
                "schema",
                "url",
                "refused",
                "failed",
                "closed",
            ]
            .into_iter()
            .filter(|term| message.contains(*term))
            .collect::<Vec<_>>()
        }
        _ => Vec::new(),
    };
    crate::logging::warn_with_fields(
        "provider.checkpoint",
        "Native checkpoint probe failed",
        json!({"provider_run_id":run.id(), "stage":stage, "operation":operation, "failure_terms":failure_terms}),
    );
    Failure::ResumeFailed
}

#[cfg(not(unix))]
pub(super) fn export(_: &RuntimeProviderRun) -> Result<ProviderNativeCheckpoint, Failure> {
    Err(Failure::UnsupportedPlatform)
}
#[cfg(not(unix))]
pub(super) fn import(
    _: &ProviderNativeCheckpoint,
    _: &RuntimeProviderRun,
) -> Result<ProviderResumeState, Failure> {
    Err(Failure::UnsupportedPlatform)
}

#[cfg(not(unix))]
pub(super) fn import_for_move(
    _: &ProviderNativeCheckpoint,
    _: &RuntimeProviderRun,
) -> Result<ProviderResumeState, Failure> {
    Err(Failure::UnsupportedPlatform)
}

pub(super) fn verify_required_checkpoint(
    client: &CodexClient,
    run: &RuntimeProviderRun,
    binding: &NativeCheckpointBinding,
) -> Result<(), Failure> {
    let auth = auth(client, run.account_profile())?;
    if account_digest(&auth)? != binding.account_sha256 {
        return Err(Failure::AccountMismatch);
    }
    #[cfg(unix)]
    {
        let metadata = metadata(client, &binding.session_id).map_err(|_| Failure::ResumeFailed)?;
        idle(&metadata, &binding.session_id)?;
        let root = profile_root(run.pty_env())?;
        let relative = metadata
            .path
            .as_deref()
            .and_then(|path| path.strip_prefix(root).ok())
            .and_then(Path::to_str)
            .ok_or(Failure::TargetConflict)?;
        verify_required_artifact(&super::files::Directory::open(root)?, relative, binding)?;
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn verify_required_artifact(
    directory: &super::files::Directory,
    relative: &str,
    binding: &NativeCheckpointBinding,
) -> Result<(), Failure> {
    if !valid_relative_path(relative, &binding.session_id) {
        return Err(Failure::TargetConflict);
    }
    let contents = directory.read(relative)?;
    // Official thread/resume appends native events before the first user turn.
    // A Move readiness probe can resume this same thread before its first run.
    // Re-import and continuity both retain the complete imported checkpoint
    // as a prefix ending at a JSONL record boundary.
    let mut hash = Sha256::new();
    for record in contents.split_inclusive(|byte| *byte == b'\n') {
        hash.update(record);
        if format!("{:x}", hash.clone().finalize()) == binding.artifact_sha256 {
            return Ok(());
        }
    }
    Err(Failure::TargetConflict)
}

#[cfg(test)]
mod tests {
    #[test]
    fn mk03_native_checkpoint_probe_logs_static_failure_terms() {
        let capture = crate::logging::capture::start();
        let request = crate::provider::LaunchProviderRequest::new(
            "room",
            "codex",
            "codex",
            "profile",
            "gpt-6-luna",
        );
        let run = crate::provider::RuntimeProviderRun::new(
            "provider-run-1",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "test".into(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: Default::default(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        let error = crate::DaemonError::ProviderProtocol {
            provider_run_id: "provider-run-1".into(),
            operation: "thread/resume",
            message: "missing field `model`; private-provider-value".into(),
        };
        assert_eq!(
            super::resume_probe_failure(&run, "thread/resume", &error),
            super::Failure::ResumeFailed
        );
        let records = capture.records();
        let record = records
            .iter()
            .find(|row| row.contains("Native checkpoint probe failed"))
            .unwrap();
        assert!(
            record.contains("missing field"),
            "probe must retain a static failure classification"
        );
        assert!(record.contains("model"));
        assert!(!record.contains("private-provider-value"));
    }
}
