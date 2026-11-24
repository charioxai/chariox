//! Claude's own project-trust record for the workspace Chariox chose for an agent.
//!
//! The person or orchestrating agent picked that folder when the agent was
//! created, so asking the person to trust it again is a second question about
//! the same decision. Only the chosen workspace is ever recorded.
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::error::DaemonError;
use crate::provider::LaunchProviderRequest;

const OPERATION: &str = "trust the Claude workspace";

/// Shown in the Room when a launch trusts the agent's folder, so the person sees it happen.
pub(crate) fn trusted_workspace_notice(workspace: &Path) -> String {
    format!("Chariox trusted {} for Claude Code.", workspace.display())
}

/// What a launch writes into Claude's state file.
pub(super) struct ClaudeStateUpdate<'a> {
    pub(super) onboarding: bool,
    pub(super) trusted_workspace: Option<&'a Path>,
}

/// What an update changed in the state file.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct StateChange {
    pub(super) changed: bool,
    /// The workspace was not trusted before this update.
    pub(super) trusted: bool,
}

impl ClaudeStateUpdate<'_> {
    #[cfg(test)]
    pub(super) fn onboarding_only() -> Self {
        Self {
            onboarding: true,
            trusted_workspace: None,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        !self.onboarding && self.trusted_workspace.is_none()
    }

    pub(super) fn apply(&self, state: &mut Map<String, Value>) -> Result<StateChange, DaemonError> {
        let mut change = StateChange::default();
        if self.onboarding
            && state.get("hasCompletedOnboarding").and_then(Value::as_bool) != Some(true)
        {
            state.insert("hasCompletedOnboarding".into(), Value::Bool(true));
            change.changed = true;
        }
        if let Some(workspace) = self.trusted_workspace {
            change.trusted = record_workspace_trust(state, workspace)?;
            change.changed |= change.trusted;
        }
        Ok(change)
    }
}

/// The workspace this launch may trust, or `None` when trusting it would cover
/// more than the one folder: the profile's home directory or a filesystem root
/// holds other projects, so Claude's own question stays a real choice there.
/// A launch directory other than the chosen workspace is refused: trust is
/// never given to a folder nobody chose for this agent.
pub(super) fn workspace_to_trust(
    request: &LaunchProviderRequest,
    chosen_workspace: &Path,
) -> Result<Option<PathBuf>, DaemonError> {
    let launch_directory = request.working_directory.as_deref().ok_or_else(|| {
        refused("the launch has no working directory; start the agent from its workspace")
    })?;
    let chosen = canonical(chosen_workspace)?;
    if canonical(launch_directory)? != chosen {
        return Err(refused(format!(
            "the launch directory {} is not the agent's workspace {}; start the agent from its workspace",
            launch_directory.display(),
            chosen.display()
        )));
    }
    let profile_home = home(request).and_then(|home| home.canonicalize().ok());
    if chosen.parent().is_none() || profile_home.as_ref() == Some(&chosen) {
        return Ok(None);
    }
    Ok(Some(chosen))
}

fn home(request: &LaunchProviderRequest) -> Option<PathBuf> {
    request
        .provider_account_env
        .get("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            request
                .provider_credential_env
                .iter()
                .find(|(name, value)| *name == "HOME" && !value.is_empty())
                .map(|(_, value)| PathBuf::from(value))
        })
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
}

/// Claude keys projects by the absolute path it runs in, symlinks resolved.
fn canonical(path: &Path) -> Result<PathBuf, DaemonError> {
    path.canonicalize().map_err(|error| {
        refused(format!(
            "workspace {} is unavailable: {error}",
            path.display()
        ))
    })
}

fn record_workspace_trust(
    state: &mut Map<String, Value>,
    workspace: &Path,
) -> Result<bool, DaemonError> {
    let workspace = workspace
        .to_str()
        .ok_or_else(|| refused("the workspace path is not UTF-8"))?;
    let projects = object_entry(state, "projects")?;
    let project = object_entry(projects, workspace)?;
    if project
        .get("hasTrustDialogAccepted")
        .and_then(Value::as_bool)
        == Some(true)
    {
        return Ok(false);
    }
    project.insert("hasTrustDialogAccepted".into(), Value::Bool(true));
    Ok(true)
}

/// A value Claude wrote with another shape is never replaced: the launch fails
/// and says which entry to fix.
fn object_entry<'a>(
    parent: &'a mut Map<String, Value>,
    key: &str,
) -> Result<&'a mut Map<String, Value>, DaemonError> {
    parent
        .entry(key)
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| {
            refused(format!(
                "Claude state entry `{key}` is not a JSON object; fix or remove it, then start the agent again"
            ))
        })
}

fn refused(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: OPERATION,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn trust_changes_only_the_workspace_flag_and_is_idempotent() {
        let mut claude = state(json!({
            "theme": "dark",
            "projects": {
                "/work/a": {"allowedTools": ["Bash"], "hasTrustDialogAccepted": false},
                "/work/b": {"hasTrustDialogAccepted": true}
            }
        }));
        let update = ClaudeStateUpdate {
            onboarding: false,
            trusted_workspace: Some(Path::new("/work/a")),
        };
        assert!(update.apply(&mut claude).unwrap().trusted);
        assert_eq!(
            Value::Object(claude.clone()),
            json!({
                "theme": "dark",
                "projects": {
                    "/work/a": {"allowedTools": ["Bash"], "hasTrustDialogAccepted": true},
                    "/work/b": {"hasTrustDialogAccepted": true}
                }
            })
        );
        assert!(
            !update.apply(&mut claude).unwrap().changed,
            "already trusted"
        );
    }

    #[test]
    fn a_state_entry_of_the_wrong_shape_fails_the_launch_and_is_not_replaced() {
        for (wrong, entry) in [
            (json!({"projects": []}), "projects"),
            (json!({"projects": {"/work/a": true}}), "/work/a"),
        ] {
            let mut claude = state(wrong.clone());
            let error = ClaudeStateUpdate {
                onboarding: false,
                trusted_workspace: Some(Path::new("/work/a")),
            }
            .apply(&mut claude)
            .unwrap_err();
            assert!(error.to_string().contains(entry), "{error}");
            assert_eq!(Value::Object(claude), wrong);
        }
    }

    #[test]
    fn only_the_chosen_workspace_below_home_is_trusted() {
        let root = crate::test_support::TestWorktree::new("claude-trust-scope");
        let home = root.path().join("home");
        let chosen = home.join("project");
        std::fs::create_dir_all(&chosen).unwrap();
        let request = |directory: &Path| {
            let mut credentials = crate::provider::ProviderCredentialEnvironment::default();
            credentials.insert("HOME", zeroize::Zeroizing::new(home.display().to_string()));
            LaunchProviderRequest::new("session", "claude", "claude-headless", "default", "haiku")
                .with_provider_credential_env(credentials)
                .with_working_directory(directory.to_path_buf())
        };
        assert_eq!(
            workspace_to_trust(&request(&chosen), &chosen).unwrap(),
            Some(chosen.canonicalize().unwrap())
        );
        assert_eq!(
            workspace_to_trust(&request(&home), &home).unwrap(),
            None,
            "the home directory holds other projects"
        );
        assert_eq!(
            workspace_to_trust(&request(Path::new("/")), Path::new("/")).unwrap(),
            None,
            "a filesystem root holds everything"
        );
        let error = workspace_to_trust(&request(&home), &chosen).unwrap_err();
        assert!(error.to_string().contains("not the agent's workspace"));
    }
}
