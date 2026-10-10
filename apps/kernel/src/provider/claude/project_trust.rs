//! MP-08 / MP-10 / MP-11: only a user-selected Chariox project receives trust.
use std::env;
use std::path::{Path, PathBuf};

use crate::error::DaemonError;
use crate::provider::LaunchProviderRequest;

use super::config_state;

pub(super) fn home(request: &LaunchProviderRequest) -> Option<PathBuf> {
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
        .or_else(|| env::var_os("HOME").map(PathBuf::from))
}

pub(super) fn state_path(request: Option<&LaunchProviderRequest>) -> Result<PathBuf, DaemonError> {
    let configured = request
        .and_then(|request| {
            request
                .provider_account_env
                .get(super::CLAUDE_CONFIG_DIR_ENV)
        })
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            if request.is_some_and(|request| {
                request
                    .provider_env_remove
                    .iter()
                    .any(|key| key == super::CLAUDE_CONFIG_DIR_ENV)
            }) {
                None
            } else {
                env::var_os(super::CLAUDE_CONFIG_DIR_ENV)
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
            }
        });
    let directory = configured
        .or_else(|| request.and_then(home))
        .or_else(|| env::var_os("HOME").map(PathBuf::from))
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "prepare Claude selected project trust",
            message: "HOME is unavailable".into(),
        })?;
    Ok(directory.join(super::CLAUDE_HEADLESS_STATE_FILE))
}

pub(crate) fn selected_project_trust_scope(selected: &Path) -> bool {
    // Claude may ask to trust an ancestor Git root or a worktree's main checkout.
    // Opening the child project never authorizes either wider scope.
    for ancestor in selected.ancestors() {
        let git = ancestor.join(".git");
        match git.symlink_metadata() {
            Ok(_) => {
                return ancestor == selected
                    && git.is_dir()
                    && git
                        .canonicalize()
                        .is_ok_and(|path| path.starts_with(selected))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return false,
        }
    }
    true
}

pub(crate) fn redirect_claude_home_to_project(
    request: &mut LaunchProviderRequest,
    selected: &Path,
    is_project: bool,
) -> Result<(), DaemonError> {
    if request.provider != super::CLAUDE_HEADLESS_PROVIDER_ID {
        return Ok(());
    }
    let Some(home) = home(request).and_then(|path| path.canonicalize().ok()) else {
        return Ok(());
    };
    let selected = selected.canonicalize().ok();
    let cwd = request
        .working_directory
        .as_deref()
        .and_then(|path| path.canonicalize().ok());
    if cwd.as_ref() != Some(&home) {
        return Ok(());
    }
    if !is_project || selected.as_ref().is_none_or(|selected| selected == &home) {
        return Err(DaemonError::LocalTransport {
            operation: "prepare Claude selected project trust",
            message: "Choose a Chariox project folder before launching Claude; the home directory cannot be trusted automatically".into(),
        });
    }
    request.working_directory = selected;
    Ok(())
}

pub(crate) fn prepare_claude_selected_project_trust(
    request: &LaunchProviderRequest,
    selected: &Path,
    is_project: bool,
) {
    if request.provider != super::CLAUDE_HEADLESS_PROVIDER_ID {
        return;
    }
    let selected = selected.canonicalize().ok();
    let cwd = request
        .working_directory
        .as_deref()
        .and_then(|path| path.canonicalize().ok());
    let home = home(request).and_then(|path| path.canonicalize().ok());
    let eligible = is_project
        && selected.is_some()
        && selected == cwd
        && home.is_some()
        && selected != home
        && selected
            .as_deref()
            .is_some_and(selected_project_trust_scope);
    let outcome = if eligible {
        state_path(Some(request))
            .and_then(|state| record_project(&state, selected.as_deref().unwrap()))
    } else {
        Err(DaemonError::LocalTransport {
            operation: "prepare Claude selected project trust",
            message: "launch does not identify a project with an exact trust scope".into(),
        })
    };
    // Never log provider config or credentials, including in a parse failure.
    crate::logging::info_with_fields(
        "daemon.provider.claude",
        "prepared Claude selected project trust",
        serde_json::json!({
            "session_id": request.session_id,
            "account_profile": request.account_profile,
            "trust_path": if outcome.is_ok() { "pre-recorded" } else { "screen-fallback" },
        }),
    );
}

fn record_project(state: &Path, project: &Path) -> Result<(), DaemonError> {
    record_project_before_commit(state, project, || {})
}

fn record_project_before_commit(
    state: &Path,
    project: &Path,
    before_commit: impl FnMut(),
) -> Result<(), DaemonError> {
    let project = project
        .to_str()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "prepare Claude selected project trust",
            message: "Claude project path is not UTF-8".into(),
        })?;
    config_state::merge(
        state,
        |root| {
            let mut projects = root
                .get("projects")
                .map(|raw| config_state::object(raw))
                .transpose()?
                .unwrap_or_default();
            let mut selected = projects
                .get(project)
                .map(|raw| config_state::object(raw))
                .transpose()?
                .unwrap_or_default();
            if selected
                .get("hasTrustDialogAccepted")
                .is_some_and(|raw| raw.get() == "true")
            {
                return Ok(false);
            }
            selected.insert("hasTrustDialogAccepted".into(), config_state::raw(&true)?);
            projects.insert(project.into(), config_state::raw(&selected)?);
            root.insert("projects".into(), config_state::raw(&projects)?);
            Ok(true)
        },
        before_commit,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_recorded_claude_trust_preserves_other_projects_and_opaque_values() {
        let root = crate::test_support::TestWorktree::new("claude-project-trust-merge");
        let state = root.path().join(".claude.json");
        let initial = r#"{"opaque":{"fixture":"unchanged"},"huge":184467440737095516160,"projects":{"/other":{"hasTrustDialogAccepted":false,"unknown":1.234567890123456789},"/selected":{"unknown":"keep"}}}"#;
        std::fs::write(&state, initial).unwrap();
        record_project(&state, Path::new("/selected")).unwrap();
        let bytes = std::fs::read(&state).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            value
                .pointer("/projects/~1selected/hasTrustDialogAccepted")
                .and_then(|v| v.as_bool())
                == Some(true)
        );
        assert!(
            value
                .pointer("/projects/~1selected/unknown")
                .and_then(|v| v.as_str())
                == Some("keep")
        );
        assert!(
            value
                .pointer("/projects/~1other/hasTrustDialogAccepted")
                .and_then(|v| v.as_bool())
                == Some(false)
        );
        assert!(String::from_utf8(bytes.clone())
            .unwrap()
            .contains("184467440737095516160"));
        assert!(String::from_utf8(bytes.clone())
            .unwrap()
            .contains("1.234567890123456789"));
        record_project(&state, Path::new("/selected")).unwrap();
        assert!(
            std::fs::read(&state).unwrap() == bytes,
            "already trusted state must not be rewritten"
        );
    }

    #[test]
    fn pre_recorded_claude_trust_never_grants_home_unchosen_or_unbound_paths() {
        crate::test_support::isolated_env_test!();
        let root = crate::test_support::TestWorktree::new("claude-trust-denials");
        let selected = root.path().join("selected");
        let other = root.path().join("other");
        let home = root.path().join("home");
        for path in [&selected, &other, &home] {
            std::fs::create_dir(path).unwrap();
        }
        let config = root.path().join("profile");
        let request =
            LaunchProviderRequest::new("session", "claude", "claude-headless", "selected", "haiku")
                .with_working_directory(selected.clone())
                .with_provider_account_env(std::collections::BTreeMap::from([
                    ("HOME".into(), home.display().to_string()),
                    ("CLAUDE_CONFIG_DIR".into(), config.display().to_string()),
                ]));
        prepare_claude_selected_project_trust(&request, &selected, false);
        let unchosen = request.clone().with_working_directory(other.clone());
        prepare_claude_selected_project_trust(&unchosen, &selected, true);
        let mut home_request = request.clone().with_working_directory(home.clone());
        prepare_claude_selected_project_trust(&home_request, &home, true);
        assert!(redirect_claude_home_to_project(&mut home_request, &home, true).is_err());
        let print = LaunchProviderRequest {
            provider: "claude-p".into(),
            ..request.clone()
        };
        prepare_claude_selected_project_trust(&print, &selected, true);
        std::fs::create_dir(root.path().join(".git")).unwrap();
        prepare_claude_selected_project_trust(&request, &selected, true);
        assert!(
            !config.join(".claude.json").exists(),
            "no rejected scope may receive trust"
        );
    }

    #[test]
    fn pre_recorded_claude_default_config_ignores_removed_ambient_profile() {
        crate::test_support::isolated_env_test!();
        let root = crate::test_support::TestWorktree::new("claude-config-selection");
        std::env::set_var("CLAUDE_CONFIG_DIR", root.path().join("ambient"));
        let request =
            LaunchProviderRequest::new("session", "claude", "claude-headless", "default", "haiku")
                .with_provider_account_env(std::collections::BTreeMap::from([(
                    "HOME".into(),
                    root.path().display().to_string(),
                )]))
                .with_provider_env_remove(vec!["CLAUDE_CONFIG_DIR".into()]);
        assert!(state_path(Some(&request)).unwrap() == root.path().join(".claude.json"));
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn pre_recorded_claude_project_trust_merges_the_latest_native_project_update() {
        let root = crate::test_support::TestWorktree::new("claude-trust-native-race");
        let state = root.path().join(".claude.json");
        std::fs::write(&state, br#"{"projects":{"/selected":{"native":"old"}}}"#).unwrap();
        let mut replaced = false;
        record_project_before_commit(&state, Path::new("/selected"), || {
            if !replaced {
                let pending = root.path().join("native-pending");
                std::fs::write(&pending, br#"{"opaque":{"fixture":"preserve"},"projects":{"/selected":{"native":"latest"},"/other":{"native":true}}}"#).unwrap();
                std::fs::rename(pending, &state).unwrap();
                replaced = true;
            }
        }).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&state).unwrap()).unwrap();
        assert!(
            value
                .pointer("/projects/~1selected/native")
                .and_then(|v| v.as_str())
                == Some("latest")
        );
        assert!(
            value
                .pointer("/projects/~1selected/hasTrustDialogAccepted")
                .and_then(|v| v.as_bool())
                == Some(true)
        );
        assert!(
            value
                .pointer("/projects/~1other/native")
                .and_then(|v| v.as_bool())
                == Some(true)
        );
        assert!(value.pointer("/opaque/fixture").and_then(|v| v.as_str()) == Some("preserve"));
    }

    #[test]
    fn pre_recorded_claude_project_trust_refuses_invalid_project_state_without_replacing_it() {
        let root = crate::test_support::TestWorktree::new("claude-trust-invalid-state");
        let state = root.path().join(".claude.json");
        for bytes in [
            b"invalid JSON".as_slice(),
            br#"{"projects":false}"#,
            br#"{"projects":{"/selected":false}}"#,
        ] {
            std::fs::write(&state, bytes).unwrap();
            assert!(record_project(&state, Path::new("/selected")).is_err());
            assert!(
                std::fs::read(&state).unwrap() == bytes,
                "unrecognized provider state must be untouched"
            );
        }
    }
}
