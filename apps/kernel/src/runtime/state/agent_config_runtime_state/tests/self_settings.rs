//! An agent that changes its own settings is not held back: the change applies at once, with no
//! refusal and no card, and is recorded with who asked and why.

use super::*;

const OWNER: &str = crate::session::DEFAULT_LOCAL_USER_ID;

fn settings_event(runtime: &KernelRuntimeState) -> serde_json::Value {
    runtime
        .owned
        .durable_state_store
        .load_events_after(0)
        .unwrap()
        .into_iter()
        .find(|event| event.kind == "agent.settings_changed")
        .expect("the change is recorded")
        .payload
}

fn assert_no_card(runtime: &KernelRuntimeState, session_id: &str) {
    let session = runtime.owned.session_store.get_session(session_id).unwrap();
    assert!(session.active_interactions().is_empty());
}

#[tokio::test]
async fn an_agent_changing_its_own_model_applies_at_once_with_no_card() {
    let (_app, runtime, session_id, agent_id) = agent_config_runtime().await;

    let (agent, _) = runtime
        .update_agent_profile_as(
            &session_id,
            &agent_id,
            OWNER,
            None,
            None,
            Some("sonnet".to_string()),
            None,
            SettingsContext::agent(&agent_id, Some("needs more depth".to_string())),
        )
        .await
        .expect("an agent may change its own model");

    assert_eq!(agent.model(), Some("sonnet"));
    assert_no_card(&runtime, &session_id);
    let event = settings_event(&runtime);
    assert_eq!(event["by"]["kind"], "agent");
    assert_eq!(event["by"]["agent_id"], agent_id);
    assert_eq!(event["reason"], "needs more depth");
    assert_eq!(event["changes"][0]["new"], "sonnet");
}

#[tokio::test]
async fn an_agent_raising_its_own_permission_level_applies_at_once_with_no_card() {
    use crate::provider::AgentPermissionLevel::{Required, Yolo};
    let (_app, runtime, session_id, agent_id) = agent_config_runtime().await;
    runtime
        .update_agent_config(
            &session_id,
            &agent_id,
            OWNER,
            None,
            Some(Some(Required)),
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let (agent, _) = runtime
        .update_agent_config_as(
            &session_id,
            &agent_id,
            OWNER,
            None,
            Some(Some(Yolo)),
            None,
            None,
            None,
            SettingsContext::agent(&agent_id, None),
        )
        .await
        .expect("an agent may change its own permission level");

    assert_eq!(agent.permission_level_override(), Some(Yolo));
    assert_no_card(&runtime, &session_id);
}

/// The room's first agent holds `Required`; its child asks for more than that.
async fn capped_child() -> (
    Arc<Mutex<DaemonApp>>,
    KernelRuntimeState,
    String,
    String,
    String,
) {
    use crate::provider::AgentPermissionLevel::Required;
    let (app, runtime, session_id, parent_id) = agent_config_runtime().await;
    runtime
        .update_agent_config(
            &session_id,
            &parent_id,
            OWNER,
            None,
            Some(Some(Required)),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let child = {
        let mut guard = app.lock().await;
        crate::app::KernelSessionService::new(&mut guard)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(&session_id, "dev-stub")
                    .with_parent_agent_id(parent_id.clone()),
            )
            .expect("child spawns")
    };
    (app, runtime, session_id, parent_id, child.id().to_string())
}

fn effective_level(
    runtime: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
) -> crate::provider::AgentPermissionLevel {
    let session = runtime.owned.session_store.get_session(session_id).unwrap();
    let agents = runtime.owned.agent_store.get_session_agents(session_id);
    let agent = runtime.owned.agent_store.get_agent(agent_id).unwrap();
    crate::session::effective_agent_execution_config_in_tree(&session, &agent, &agents)
        .permission_level
}

async fn raise_permission(
    runtime: &KernelRuntimeState,
    session_id: &str,
    child_id: &str,
    context: SettingsContext,
) -> Result<(crate::agent::AgentInstance, SettingsApplies), DaemonError> {
    runtime
        .update_agent_config_as(
            session_id,
            child_id,
            OWNER,
            None,
            Some(Some(crate::provider::AgentPermissionLevel::Yolo)),
            None,
            None,
            None,
            context,
        )
        .await
}

#[tokio::test]
async fn a_child_raising_its_permission_above_its_parent_is_refused_plainly() {
    use crate::provider::AgentPermissionLevel::Required;
    let (app, runtime, session_id, parent_id, child_id) = capped_child().await;
    sync_active_prompt(&app, &session_id, &child_id).await;

    let error = raise_permission(
        &runtime,
        &session_id,
        &child_id,
        SettingsContext::agent(&child_id, None),
    )
    .await
    .expect_err("the cap refuses an agent's own raise");

    let message = error.to_string();
    assert!(
        message.contains("limited by") && message.contains("to required"),
        "{message}"
    );
    assert_eq!(effective_level(&runtime, &session_id, &child_id), Required);
    let child = runtime.owned.agent_store.get_agent(&child_id).unwrap();
    assert_eq!(child.permission_level_override(), None);
    let parent = runtime.owned.agent_store.get_agent(&parent_id).unwrap();
    assert!(
        message.contains(parent.alias().unwrap_or_else(|| parent.agent_ref())),
        "{message}"
    );
    assert!(
        runtime
            .owned
            .terminal_stream
            .notice_records()
            .into_iter()
            .any(|notice| notice.message.contains("limited by")),
        "the pane shows the refusal"
    );
}

#[tokio::test]
async fn the_person_raising_a_childs_permission_above_its_parent_holds_after_the_turn() {
    use crate::provider::AgentPermissionLevel::Yolo;
    let (app, runtime, session_id, _parent_id, child_id) = capped_child().await;
    sync_active_prompt(&app, &session_id, &child_id).await;

    let (child, _) = raise_permission(&runtime, &session_id, &child_id, SettingsContext::person())
        .await
        .expect("the person's request is applied");

    assert_eq!(child.permission_level_override(), Some(Yolo));
    assert_eq!(effective_level(&runtime, &session_id, &child_id), Yolo);
    // The turn ends; nothing writes the old limit back.
    app.lock()
        .await
        .prompt_owner_sync_external_active_prompt(&session_id, &child_id, None)
        .unwrap();
    assert_eq!(effective_level(&runtime, &session_id, &child_id), Yolo);
    let settings = settings_event(&runtime);
    assert_eq!(settings["by"]["kind"], "person");
    assert!(runtime
        .owned
        .terminal_stream
        .notice_records()
        .into_iter()
        .any(|notice| notice.message.contains("by you")));
}

#[tokio::test]
async fn a_capped_childs_model_change_is_never_capped() {
    let (_app, runtime, session_id, _parent_id, child_id) = capped_child().await;

    let (child, _) = runtime
        .update_agent_profile_as(
            &session_id,
            &child_id,
            OWNER,
            None,
            None,
            Some("sonnet".to_string()),
            None,
            SettingsContext::agent(&child_id, None),
        )
        .await
        .expect("only permissions are capped");

    assert_eq!(child.model(), Some("sonnet"));
}
