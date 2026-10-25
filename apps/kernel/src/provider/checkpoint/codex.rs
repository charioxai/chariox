use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    digest, profile_root, NativeCheckpointBinding, NativeCheckpointFailure as Failure,
    ProviderNativeCheckpoint,
};
use crate::provider::{
    CodexClient, LaunchProviderRequest, ProviderAuthStatus, ProviderLaunchResult,
    ProviderResumeState, RuntimeProviderRun,
};

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
    request: &LaunchProviderRequest,
    launch: &ProviderLaunchResult,
) -> Result<ProviderResumeState, Failure> {
    if request.provider != "codex"
        || model(&request.model) != checkpoint.binding.model
        || variant(request.variant.as_deref()) != checkpoint.binding.variant.as_deref()
    {
        return Err(Failure::ConfigurationMismatch);
    }
    let root = profile_root(&launch.pty_env)?;
    let cwd = launch
        .working_directory
        .as_deref()
        .filter(|path| path.is_absolute() && path.is_dir())
        .ok_or(Failure::ConfigurationMismatch)?;
    let client = CodexClient::new(
        "native-checkpoint-import",
        launch
            .structured_endpoint
            .as_deref()
            .ok_or(Failure::ResumeFailed)?,
    )
    .map_err(|_| Failure::ResumeFailed)?;
    let auth = auth(&client, &request.account_profile)?;
    if account_digest(&auth)? != checkpoint.binding.account_sha256 {
        return Err(Failure::AccountMismatch);
    }
    let directory = super::files::Directory::open(root)?;
    let id = checkpoint.provider_session_id();
    match metadata(&client, id) {
        Ok(existing) => {
            idle(&existing, id)?;
            let relative = existing
                .path
                .as_deref()
                .and_then(|p| p.strip_prefix(root).ok())
                .and_then(Path::to_str)
                .ok_or(Failure::TargetConflict)?;
            if !valid_relative_path(relative, id)
                || digest(&directory.read(relative)?) != checkpoint.sha256()
            {
                return Err(Failure::TargetConflict);
            }
        }
        Err(crate::DaemonError::ProviderProtocol {
            operation: "thread/read",
            message,
            ..
        }) if message == format!("thread not loaded: {id}")
            || message == format!("no rollout found for thread id {id}") => {}
        Err(_) => return Err(Failure::ResumeFailed),
    }
    directory.publish(&checkpoint.relative_path, &checkpoint.contents)?;
    let client = client
        .with_mcp_servers(&request.mcp_servers)
        .with_provider_config_overrides(&request.provider_config_overrides);
    let client = if let Some(binding) = &request.runtime_mcp_binding {
        client.with_runtime_mcp_binding(Some(&binding.server_url), Some(&binding.auth_token))
    } else {
        client
    };
    let mut socket = client
        .connect_initialized()
        .map_err(|_| Failure::ResumeFailed)?;
    let resumed = client
        .thread_resume(
            &mut socket,
            &mut 1,
            id,
            Some(&cwd.to_string_lossy()),
            Some(model(&request.model)),
            request.write_access_mode,
            request.execution_mode.unwrap_or_default(),
            request.permission_level.unwrap_or_default(),
            None,
            &mut Vec::new(),
        )
        .map_err(|_| Failure::ResumeFailed)?;
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

#[cfg(not(unix))]
pub(super) fn export(_: &RuntimeProviderRun) -> Result<ProviderNativeCheckpoint, Failure> {
    Err(Failure::UnsupportedPlatform)
}
#[cfg(not(unix))]
pub(super) fn import(
    _: &ProviderNativeCheckpoint,
    _: &LaunchProviderRequest,
    _: &ProviderLaunchResult,
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
        if !valid_relative_path(relative, &binding.session_id)
            || digest(&super::files::Directory::open(root)?.read(relative)?)
                != binding.artifact_sha256
        {
            return Err(Failure::TargetConflict);
        }
    }
    Ok(())
}
