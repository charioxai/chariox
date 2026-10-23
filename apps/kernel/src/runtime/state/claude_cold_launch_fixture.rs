//! Test kit: a Claude agent without a live process, in a folder that ships
//! hooks, so a start path can be shown to stop at the hooks approval card.
use super::*;
use crate::provider::{AgentPermissionLevel, ProviderResumeState};
use std::future::Future;

const HOOKS: &str = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"./audit.sh"}]}]}}"#;

/// Makes `folder` ship project Claude hooks.
pub(super) fn write_hooks(folder: &std::path::Path) {
    std::fs::create_dir_all(folder.join(".claude")).unwrap();
    std::fs::write(folder.join(".claude/settings.json"), HOOKS).unwrap();
}

/// A disposable folder with project Claude hooks.
pub(super) fn hooks_folder() -> crate::test_support::TestWorktree {
    let folder = crate::test_support::TestWorktree::new("cold-claude-hooks");
    write_hooks(folder.path());
    folder
}

/// Asks before running a folder's hooks, as an agent in Required mode does.
pub(super) fn require_hook_approval(state: &KernelRuntimeState, agent_id: &str) {
    state
        .owned
        .agent_store
        .update_agent_config(
            agent_id,
            None,
            Some(Some(AgentPermissionLevel::Required)),
            None,
            None,
        )
        .unwrap();
}

/// Turns the agent into a Required-mode Claude-headless agent with no live run, placed
/// in `folder` and resuming `resume_state`.
pub(super) async fn make_cold_claude(
    state: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    folder: &std::path::Path,
    resume_state: ProviderResumeState,
) {
    // An isolated account keeps the folder's trust state out of the host's
    // own Claude profile.
    let account = state
        .owned
        .provider_account_profiles
        .create_managed(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "claude",
            "Cold launch fixture",
        )
        .unwrap()
        .profile_id;
    crate::test_support::authenticate_provider_account(
        &state.owned.provider_account_profiles,
        crate::session::DEFAULT_LOCAL_USER_ID,
        "claude",
        &account,
    )
    .unwrap();
    state
        .owned
        .agent_store
        .set_agent_runtime_profile_with_account_profile(
            agent_id,
            "claude-headless",
            Some("haiku".into()),
            None,
            Some(account),
            resume_state,
        )
        .unwrap();
    require_hook_approval(state, agent_id);
    state
        .owned
        .agent_store
        .update_agent_config(
            agent_id,
            None,
            None,
            None,
            Some(Some(folder.display().to_string())),
        )
        .unwrap();
    let (session_id, agent_id) = (session_id.to_owned(), agent_id.to_owned());
    state
        .with_app_side_effect(move |app| {
            app.end_agent_provider_run(&session_id, &agent_id)?;
            app.cold_claude_launch_for_agent(&session_id, &agent_id)?
                .expect("the fixture agent must be a cold Claude launch");
            Ok::<(), DaemonError>(())
        })
        .await
        .unwrap();
}

/// Runs `start` until the agent's hooks card appears, cancels the card, and
/// returns what `start` ends with. A start that never asks about the folder's
/// hooks fails here.
pub(super) async fn start_waits_for_hooks_card<T: std::fmt::Debug>(
    state: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    start: impl Future<Output = T>,
) -> T {
    let mut start = Box::pin(start);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let card = loop {
        tokio::select! {
            result = &mut start => panic!("the start went ahead without the hooks card: {result:?}"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => {}
        }
        if let Some(card) = state
            .owned
            .session_store
            .get_session(session_id)
            .unwrap()
            .active_interaction_for_agent(agent_id)
            .cloned()
        {
            break card;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no hooks card within 5 s"
        );
    };
    assert!(card.id().starts_with("claude-workspace-hooks-"), "{card:?}");
    state
        .resolve_terminal_runtime_interaction(
            session_id,
            card.id(),
            "cancel",
            None,
            Some(crate::session::DEFAULT_LOCAL_USER_ID),
        )
        .await
        .unwrap();
    start.await
}
