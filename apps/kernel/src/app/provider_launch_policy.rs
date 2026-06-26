use rand::distributions::{Alphanumeric, DistString};
use std::path::{Path, PathBuf};

use crate::agent::AgentInstance;
use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::mcp::ArrobaMcpServerConfig;
use crate::provider::{
    AgentExecutionMode, LaunchProviderRequest, ProviderResumeState, RuntimeProviderRun,
};
use crate::session::RuntimeSession;

pub(super) fn default_provider_env_remove(config: &DaemonConfig) -> Vec<String> {
    let credentials = crate::credential::load_user_credentials().unwrap_or_default();
    let _ = config;
    crate::secret::RuntimeSecretService::credential_env_names_from(&credentials)
        .into_iter()
        .collect()
}

pub(crate) fn resolve_mcp_credentials_for_launch(
    config: &DaemonConfig,
    servers: Vec<ArrobaMcpServerConfig>,
) -> Result<Vec<ArrobaMcpServerConfig>, DaemonError> {
    if servers.is_empty() {
        return Ok(servers);
    }
    let credentials = crate::credential::load_user_credentials()?;
    let service = crate::secret::RuntimeSecretService::with_vault_config(
        credentials,
        &config.user_config.credential_vault,
    )?;
    servers
        .into_iter()
        .map(|server| server.resolve_credential_bindings(&service))
        .collect()
}

pub(crate) fn sanitize_resume_state_for_launch(
    request: &LaunchProviderRequest,
    agent: &AgentInstance,
) -> ProviderResumeState {
    let resume_state = agent.provider_resume_state().clone();
    let requested_variant = request
        .variant
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let agent_variant = agent.effort().filter(|value| !value.trim().is_empty());
    let requested_model =
        normalize_resume_model_for_adapter(&request.adapter_key, request.model.as_str());
    let agent_model = agent
        .model()
        .map(|model| normalize_resume_model_for_adapter(&request.adapter_key, model));
    let model_or_variant_changed = agent_model.as_deref() != Some(requested_model.as_str())
        || agent_variant != requested_variant;
    if !model_or_variant_changed {
        return resume_state;
    }

    match request.adapter_key.as_str() {
        "opencode" => resume_state.without_opencode_session_id(),
        "codex" => resume_state.without_codex_thread_id(),
        "claude" => resume_state.without_claude_session_id(),
        _ => resume_state,
    }
}

pub(crate) fn granted_mcp_servers_for_agent_launch(
    operation: &'static str,
    session: &RuntimeSession,
    agent: &AgentInstance,
) -> Result<Vec<ArrobaMcpServerConfig>, DaemonError> {
    let mcp_grants = agent.mcp_grants();
    if mcp_grants.is_empty() {
        return Ok(Vec::new());
    }
    let roots = crate::mcp::ArrobaMcpRegistry::user_root()
        .map(|root| vec![root])
        .unwrap_or_default();
    let registry = crate::mcp::ArrobaMcpRegistry::new(roots);
    let mut servers = Vec::new();
    for grant in mcp_grants {
        let Some(server) = registry.get(&grant)? else {
            crate::logging::warn_with_fields(
                "daemon.provider",
                "skipping missing MCP extension grant during provider launch",
                serde_json::json!({
                    "operation": operation,
                    "session_id": session.id(),
                    "agent_id": agent.id(),
                    "agent_ref": agent.agent_ref(),
                    "mcp": grant,
                }),
            );
            continue;
        };
        if server.enabled {
            servers.push(server);
        }
    }
    Ok(servers)
}

pub(crate) fn apply_metaagent_launch_policy(
    mut request: LaunchProviderRequest,
    agent: Option<&AgentInstance>,
) -> LaunchProviderRequest {
    if !agent.is_some_and(AgentInstance::is_metaagent) {
        return request;
    }
    request = request
        .with_execution_mode(AgentExecutionMode::Plan)
        .with_mcp_servers(Vec::new())
        .with_remote_extension_manifest(crate::extension::RemoteExtensionManifest::default());
    request
}

pub(crate) fn failed_codex_resume_state_replacement(
    run: &RuntimeProviderRun,
    error: &DaemonError,
) -> Option<ProviderResumeState> {
    if run.adapter_key() != "codex" || run.resume_state().codex_thread_id().is_none() {
        return None;
    }
    let DaemonError::ProviderProtocol { operation, .. } = error else {
        return None;
    };
    if *operation != "codex_thread_resume" {
        return None;
    }
    Some(run.resume_state().without_codex_thread_id())
}

pub(crate) fn generate_runtime_mcp_auth_token() -> String {
    Alphanumeric.sample_string(&mut rand::thread_rng(), 32)
}

pub(crate) fn workspace_live_sync_protected_roots(
    session: &RuntimeSession,
    working_directory: Option<&Path>,
    host_machine_id: &str,
    host_daemon_id: &str,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = working_directory
        .and_then(resolve_git_root)
        .or_else(|| working_directory.map(PathBuf::from))
    {
        push_unique_root(&mut roots, root);
    }
    for link in session.workspace_links() {
        for attachment in link.attachments() {
            if attachment.machine_id() == host_machine_id
                && attachment.kernel_id() == host_daemon_id
            {
                push_unique_root(&mut roots, PathBuf::from(attachment.repo_root()));
            }
        }
    }
    roots
}

fn resolve_git_root(path: &Path) -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8(output.stdout).ok()?;
    let root = root.trim();
    (!root.is_empty()).then(|| PathBuf::from(root))
}

fn push_unique_root(roots: &mut Vec<PathBuf>, root: PathBuf) {
    if !roots.iter().any(|existing| existing == &root) {
        roots.push(root);
    }
}

fn normalize_resume_model_for_adapter(adapter_key: &str, model: &str) -> String {
    let trimmed = model.trim();
    if adapter_key == "codex" {
        trimmed
            .strip_prefix("codex/")
            .unwrap_or(trimmed)
            .to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_live_sync_protected_roots_include_working_directory_and_local_links() {
        let mut session = crate::session::RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "/repo/main",
            "machine-1",
            "daemon-1",
        );
        session.create_workspace_link(crate::session::WorkspaceLinkDefinition::new(
            "link-1",
            "session-1",
            "shared",
            "local",
        ));
        session
            .workspace_link_mut("link-1")
            .expect("link should exist")
            .attach(crate::session::WorkspaceLinkAttachment::new(
                "link-1",
                "local",
                "machine-1",
                "daemon-1",
                "/repo/attached",
                None,
                None,
            ));
        session
            .workspace_link_mut("link-1")
            .expect("link should exist")
            .attach(crate::session::WorkspaceLinkAttachment::new(
                "link-1",
                "peer",
                "remote-machine",
                "remote-daemon",
                "/remote/repo",
                None,
                None,
            ));

        let roots = workspace_live_sync_protected_roots(
            &session,
            Some(Path::new("/repo/main")),
            "machine-1",
            "daemon-1",
        );

        assert_eq!(
            roots,
            vec![PathBuf::from("/repo/main"), PathBuf::from("/repo/attached"),]
        );
    }

    #[test]
    fn workspace_live_sync_protected_roots_do_not_include_sibling_repos() {
        let base = std::env::temp_dir().join(format!(
            "arroba-live-sync-root-scope-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let selected = base.join("selected");
        let selected_child = selected.join("src");
        let sibling = base.join("sibling");
        std::fs::create_dir_all(&selected_child).expect("selected repo fixture should exist");
        std::fs::create_dir_all(&sibling).expect("sibling repo fixture should exist");
        run_git_init(&selected);
        run_git_init(&sibling);
        let session = crate::session::RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            selected_child.to_string_lossy().to_string(),
            "machine-1",
            "daemon-1",
        );

        let roots = workspace_live_sync_protected_roots(
            &session,
            Some(selected_child.as_path()),
            "machine-1",
            "daemon-1",
        );

        let canonical_selected = selected
            .canonicalize()
            .expect("selected repo should canonicalize");
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(roots, vec![canonical_selected]);
    }

    fn run_git_init(path: &Path) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("init")
            .arg("-b")
            .arg("main")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("git init should run");
        assert!(
            status.success(),
            "git init should succeed in {}",
            path.display()
        );
    }
}
