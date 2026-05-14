use crate::error::DaemonError;
use crate::local::{UserConfigMutationEffect, UserConfigProviderReloadSummary};
use crate::runtime::state::{KernelRuntimeState, ProviderReloadOutcome, ProviderReloadTrigger};

pub(crate) enum UserConfigMutation {
    Set { path: String, value: String },
    Unset { path: String },
}

pub(crate) fn summarize_provider_reload_outcomes(
    outcomes: &[ProviderReloadOutcome],
) -> UserConfigProviderReloadSummary {
    let mut summary = UserConfigProviderReloadSummary {
        reloaded: 0,
        deferred: 0,
        unaffected: 0,
    };
    for outcome in outcomes {
        match outcome {
            ProviderReloadOutcome::Reloaded => summary.reloaded += 1,
            ProviderReloadOutcome::Deferred => summary.deferred += 1,
            ProviderReloadOutcome::Unaffected => summary.unaffected += 1,
        }
    }
    summary
}

pub(crate) async fn user_config_mutation_effects(
    runtime_state: &KernelRuntimeState,
    path: &str,
) -> Result<Vec<UserConfigMutationEffect>, DaemonError> {
    if path == "providers.managed_io" {
        let outcomes = runtime_state
            .apply_provider_reload_policy(ProviderReloadTrigger::UserConfigChanged {
                path: path.to_string(),
            })
            .await?;
        let summary = summarize_provider_reload_outcomes(&outcomes);
        let message = if summary.reloaded == 0 && summary.deferred == 0 {
            "managed I/O policy updated; no running provider needed reload".to_string()
        } else {
            format!(
                "managed I/O policy updated; provider reloads: {} reloaded, {} deferred, {} unaffected",
                summary.reloaded, summary.deferred, summary.unaffected
            )
        };
        return Ok(vec![UserConfigMutationEffect {
            kind: "provider_reload".to_string(),
            path: path.to_string(),
            message,
            provider_reload: Some(summary),
        }]);
    }

    if user_config_path_requires_daemon_restart(path) {
        return Ok(vec![UserConfigMutationEffect {
            kind: "restart_required".to_string(),
            path: path.to_string(),
            message: format!("`{path}` was updated; restart the daemon for it to take effect"),
            provider_reload: None,
        }]);
    }

    if user_config_path_is_unwired(path) {
        return Ok(vec![UserConfigMutationEffect {
            kind: "no_runtime_effect".to_string(),
            path: path.to_string(),
            message: format!(
                "`{path}` was updated, but this key is not currently wired to runtime behavior"
            ),
            provider_reload: None,
        }]);
    }

    Ok(Vec::new())
}

pub(crate) fn user_config_path_requires_daemon_restart(path: &str) -> bool {
    matches!(
        path,
        "history.operational.backend"
            | "history.operational.path"
            | "state.backend"
            | "state.path"
            | "kernel.websocket_host"
            | "kernel.websocket_port"
            | "kernel.runtime_mcp_host"
            | "kernel.runtime_mcp_port"
    )
}

pub(crate) fn user_config_path_is_unwired(path: &str) -> bool {
    matches!(
        path,
        "providers.default"
            | "providers.model"
            | "providers.account_profile"
            | "providers.effort"
            | "ui.theme"
            | "ui.multi_agent_response_layout"
            | "ui.max_agents_per_screen"
            | "relay.url"
            | "relay.accept_remote_leases"
            | "history.operational.retention_days"
            | "history.operational.max_size_mb"
            | "history.operational.keep_pinned_sessions"
            | "history.operational.archive_inactive_after_days"
            | "history.operational.archive_deleted_agents"
            | "history.archive.archive_deleted_agents"
            | "history.archive.archive_before_delete"
            | "history.archive.delete_operational_after_verified_archive"
            | "artifacts.operational.retention_days"
            | "slices.linux.idle_timeout_minutes"
    ) || path.starts_with("ui.worktree_aliases.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_config_policy_summarizes_provider_reload_outcomes() {
        let summary = summarize_provider_reload_outcomes(&[
            ProviderReloadOutcome::Reloaded,
            ProviderReloadOutcome::Deferred,
            ProviderReloadOutcome::Unaffected,
            ProviderReloadOutcome::Reloaded,
        ]);

        assert_eq!(summary.reloaded, 2);
        assert_eq!(summary.deferred, 1);
        assert_eq!(summary.unaffected, 1);
    }

    #[test]
    fn user_config_policy_identifies_restart_paths() {
        assert!(user_config_path_requires_daemon_restart("state.path"));
        assert!(user_config_path_requires_daemon_restart(
            "kernel.runtime_mcp_port"
        ));
        assert!(!user_config_path_requires_daemon_restart(
            "providers.default"
        ));
        assert!(!user_config_path_requires_daemon_restart("ui.theme"));
    }

    #[test]
    fn user_config_policy_identifies_unwired_paths() {
        assert!(user_config_path_is_unwired("providers.default"));
        assert!(user_config_path_is_unwired("ui.worktree_aliases.repo"));
        assert!(user_config_path_is_unwired(
            "history.archive.archive_before_delete"
        ));
        assert!(!user_config_path_is_unwired("mcp.servers"));
        assert!(!user_config_path_is_unwired("kernel.runtime_mcp_port"));
    }
}
