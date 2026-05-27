use std::collections::BTreeMap;
#[cfg(target_os = "macos")]
use std::fs;
#[cfg(target_os = "macos")]
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
#[cfg(target_os = "macos")]
use std::path::PathBuf;

use crate::error::DaemonError;

use super::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun};

pub(crate) const WORKSPACE_WRITE_FENCE_ENV: &str = "ARROBA_WORKSPACE_WRITE_FENCE";
pub(crate) const MACOS_SEATBELT_BACKEND: &str = "macos-seatbelt";

pub(crate) fn apply_workspace_write_fence(
    mut launch: ProviderLaunchResult,
    request: &LaunchProviderRequest,
) -> Result<ProviderLaunchResult, DaemonError> {
    if !request.requires_workspace_live_sync() || launch.endpoint_mode != AgentEndpointMode::Managed {
        return Ok(launch);
    }

    let Some(workspace_root) = launch.working_directory.clone() else {
        return Err(DaemonError::LocalTransport {
            operation: "workspace_write_fence",
            message: "workspace live sync provider runs require a workspace working directory".to_string(),
        });
    };
    let Some(program) = launch.pty_program.clone() else {
        return Err(DaemonError::LocalTransport {
            operation: "workspace_write_fence",
            message: "workspace live sync provider runs require an Arroba-owned provider process"
                .to_string(),
        });
    };

    apply_platform_workspace_write_fence(&mut launch, request, &workspace_root, program)?;
    Ok(launch)
}

pub(crate) fn workspace_write_fence_active(run: &RuntimeProviderRun) -> bool {
    workspace_write_fence_active_env(run.pty_env())
}

pub(crate) fn workspace_write_fence_active_env(env: &BTreeMap<String, String>) -> bool {
    env.get(WORKSPACE_WRITE_FENCE_ENV)
        .is_some_and(|value| value == MACOS_SEATBELT_BACKEND)
}

#[cfg(target_os = "macos")]
fn apply_platform_workspace_write_fence(
    launch: &mut ProviderLaunchResult,
    request: &LaunchProviderRequest,
    workspace_root: &Path,
    program: String,
) -> Result<(), DaemonError> {
    let sandbox_exec = Path::new("/usr/bin/sandbox-exec");
    if !sandbox_exec.exists() {
        return Err(DaemonError::LocalTransport {
            operation: "workspace_write_fence",
            message: "macOS workspace write fence requires /usr/bin/sandbox-exec".to_string(),
        });
    }

    let canonical_workspace =
        workspace_root
            .canonicalize()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "workspace_write_fence",
                message: format!(
                    "failed to canonicalize workspace `{}`: {error}",
                    workspace_root.display()
                ),
            })?;
    let profile_path = write_macos_seatbelt_profile(request, &canonical_workspace)?;

    let mut wrapped_args = vec![
        "-f".to_string(),
        profile_path.display().to_string(),
        program,
    ];
    wrapped_args.append(&mut launch.pty_args);

    launch.pty_program = Some(sandbox_exec.display().to_string());
    launch.pty_args = wrapped_args;
    launch.pty_env.insert(
        WORKSPACE_WRITE_FENCE_ENV.to_string(),
        MACOS_SEATBELT_BACKEND.to_string(),
    );
    launch.process_label = format!("{}:fenced", launch.process_label);
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn apply_platform_workspace_write_fence(
    launch: &mut ProviderLaunchResult,
    _request: &LaunchProviderRequest,
    _workspace_root: &Path,
    _program: String,
) -> Result<(), DaemonError> {
    let _ = launch;
    Ok(())
}

#[cfg(target_os = "macos")]
fn write_macos_seatbelt_profile(
    request: &LaunchProviderRequest,
    canonical_workspace: &Path,
) -> Result<PathBuf, DaemonError> {
    let profile_dir = std::env::temp_dir().join("arroba-workspace-write-fences");
    fs::create_dir_all(&profile_dir).map_err(|error| DaemonError::LocalTransport {
        operation: "workspace_write_fence",
        message: format!(
            "failed to create workspace write fence profile directory `{}`: {error}",
            profile_dir.display()
        ),
    })?;
    let profile_path = profile_dir.join(format!(
        "workspace-{:x}.sb",
        workspace_fence_hash(request, canonical_workspace)
    ));
    let profile = macos_seatbelt_profile(canonical_workspace);
    fs::write(&profile_path, profile).map_err(|error| DaemonError::LocalTransport {
        operation: "workspace_write_fence",
        message: format!(
            "failed to write workspace write fence profile `{}`: {error}",
            profile_path.display()
        ),
    })?;
    Ok(profile_path)
}

#[cfg(target_os = "macos")]
fn workspace_fence_hash(request: &LaunchProviderRequest, canonical_workspace: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    canonical_workspace.hash(&mut hasher);
    request.session_id.hash(&mut hasher);
    request.agent_id.hash(&mut hasher);
    request.provider.hash(&mut hasher);
    request.model.hash(&mut hasher);
    hasher.finish()
}

#[cfg(target_os = "macos")]
fn macos_seatbelt_profile(canonical_workspace: &Path) -> String {
    format!(
        "(version 1)\n(deny file-write* (subpath \"{}\"))\n(allow default)\n",
        seatbelt_string(canonical_workspace)
    )
}

#[cfg(target_os = "macos")]
fn seatbelt_string(path: &Path) -> String {
    let mut escaped = String::new();
    for ch in path.display().to_string().chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};

    use super::{
        apply_workspace_write_fence, workspace_write_fence_active_env, MACOS_SEATBELT_BACKEND,
        WORKSPACE_WRITE_FENCE_ENV,
    };

    #[test]
    fn detects_active_workspace_write_fence_env() {
        let mut env = BTreeMap::new();
        assert!(!workspace_write_fence_active_env(&env));

        env.insert(
            WORKSPACE_WRITE_FENCE_ENV.to_string(),
            MACOS_SEATBELT_BACKEND.to_string(),
        );
        assert!(workspace_write_fence_active_env(&env));
    }

    #[test]
    fn unrestricted_launch_is_not_wrapped() {
        let request =
            LaunchProviderRequest::new("session-1", "opencode", "opencode", "default", "model");
        let launch = ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "opencode:serve".to_string(),
            pty_target: None,
            pty_program: Some("/bin/echo".to_string()),
            pty_args: vec!["hello".to_string()],
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: Some(std::env::temp_dir()),
            structured_endpoint: Some("http://127.0.0.1:1".to_string()),
        };

        let wrapped =
            apply_workspace_write_fence(launch.clone(), &request).expect("launch should resolve");

        assert_eq!(wrapped, launch);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn workspace_live_sync_launch_is_wrapped_with_macos_seatbelt() {
        let workspace = std::env::temp_dir().join(format!(
            "arroba-workspace-write-fence-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&workspace).expect("workspace fixture should exist");
        let request =
            LaunchProviderRequest::new("session-1", "opencode", "opencode", "default", "model")
                .with_workspace_live_sync_required()
                .with_working_directory(workspace.clone());
        let launch = ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "opencode:serve".to_string(),
            pty_target: None,
            pty_program: Some("/bin/echo".to_string()),
            pty_args: vec!["hello".to_string()],
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: Some(workspace.clone()),
            structured_endpoint: Some("http://127.0.0.1:1".to_string()),
        };

        let wrapped =
            apply_workspace_write_fence(launch, &request).expect("launch should be wrapped");

        let _ = std::fs::remove_dir_all(&workspace);
        assert_eq!(
            wrapped.pty_program.as_deref(),
            Some("/usr/bin/sandbox-exec")
        );
        assert_eq!(wrapped.pty_args[0], "-f");
        assert!(wrapped.pty_args[1].contains("arroba-workspace-write-fences"));
        assert_eq!(wrapped.pty_args[2], "/bin/echo");
        assert_eq!(wrapped.pty_args[3], "hello");
        assert_eq!(
            wrapped
                .pty_env
                .get(WORKSPACE_WRITE_FENCE_ENV)
                .map(String::as_str),
            Some(MACOS_SEATBELT_BACKEND)
        );
        assert!(wrapped.process_label.ends_with(":fenced"));
    }
}
