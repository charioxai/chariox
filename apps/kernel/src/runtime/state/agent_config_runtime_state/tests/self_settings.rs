//! An agent that changes its own settings stays within what the person allowed.

use super::*;
use crate::agent::AgentSelfSettings;

const OWNER: &str = crate::session::DEFAULT_LOCAL_USER_ID;

async fn allow(
    runtime: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    set: AgentSelfSettings,
) {
    runtime
        .update_agent_config_as(
            session_id,
            agent_id,
            OWNER,
            None,
            None,
            None,
            None,
            None,
            Some(set),
            SettingsContext::person(),
        )
        .await
        .expect("the person sets what the agent may change");
}

async fn agent_changes_model(
    runtime: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
    model: &str,
) -> Result<crate::agent::AgentInstance, DaemonError> {
    runtime
        .update_agent_profile_as(
            session_id,
            agent_id,
            OWNER,
            None,
            None,
            Some(model.to_string()),
            None,
            SettingsContext::agent(agent_id, Some("needs more depth".to_string())),
        )
        .await
        .map(|(agent, _)| agent)
}

/// The card the agent's call waits behind, once it is up.
async fn waiting_card(
    runtime: &KernelRuntimeState,
    session_id: &str,
    agent_id: &str,
) -> (String, Vec<String>, String) {
    for _ in 0..200 {
        let session = runtime.owned.session_store.get_session(session_id).unwrap();
        if let Some(card) = session
            .active_interactions()
            .iter()
            .find(|card| card.related_agent_id() == Some(agent_id))
        {
            return (
                card.id().to_string(),
                card.choices()
                    .iter()
                    .map(|choice| choice.id().to_string())
                    .collect(),
                card.message().to_string(),
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the person's card never appeared");
}

#[tokio::test]
async fn a_change_inside_the_allowed_set_applies_at_once_with_no_card() {
    let (_app, runtime, session_id, agent_id) = agent_config_runtime().await;
    allow(
        &runtime,
        &session_id,
        &agent_id,
        AgentSelfSettings {
            models: vec!["sonnet".into()],
            ..Default::default()
        },
    )
    .await;

    let agent = agent_changes_model(&runtime, &session_id, &agent_id, "sonnet")
        .await
        .expect("inside the set");

    assert_eq!(agent.model(), Some("sonnet"));
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    assert!(session.active_interactions().is_empty());
    let event = runtime
        .owned
        .durable_state_store
        .load_events_after(0)
        .unwrap()
        .into_iter()
        .find(|event| event.kind == "agent.settings_changed")
        .expect("the change is recorded")
        .payload;
    assert_eq!(event["by"]["kind"], "agent");
    assert_eq!(event["by"]["agent_id"], agent_id);
    assert_eq!(event["reason"], "needs more depth");
    assert_eq!(event["changes"][0]["new"], "sonnet");
}

#[tokio::test]
async fn a_change_outside_the_set_takes_one_card_and_allow_always_remembers_it() {
    let (_app, runtime, session_id, agent_id) = agent_config_runtime().await;
    let task = {
        let (runtime, session_id, agent_id) =
            (runtime.clone(), session_id.clone(), agent_id.clone());
        tokio::spawn(
            async move { agent_changes_model(&runtime, &session_id, &agent_id, "opus").await },
        )
    };

    let (card, choices, message) = waiting_card(&runtime, &session_id, &agent_id).await;
    assert_eq!(choices, ["allow_once", "allow_always", "deny"]);
    assert!(
        message.contains("wants to change its model to opus"),
        "{message}"
    );
    assert!(message.contains("needs more depth"), "{message}");
    assert_ne!(
        runtime
            .owned
            .agent_store
            .get_agent(&agent_id)
            .unwrap()
            .model(),
        Some("opus"),
        "nothing changes before the person answers"
    );
    runtime
        .resolve_terminal_runtime_interaction(&session_id, &card, "allow_always", None, Some(OWNER))
        .await
        .unwrap();
    let agent = task.await.unwrap().expect("allowed");

    assert_eq!(agent.model(), Some("opus"));
    assert!(agent
        .self_settings()
        .allows(crate::agent::SelfSettingKind::Model, "opus"));
    // Standing permission: the same change needs no second card.
    agent_changes_model(&runtime, &session_id, &agent_id, "sonnet-then")
        .await
        .ok();
    let again = agent_changes_model(&runtime, &session_id, &agent_id, "opus").await;
    assert!(again.is_ok());
}

#[tokio::test]
async fn denying_the_card_leaves_the_agent_as_it_was_and_says_why() {
    let (_app, runtime, session_id, agent_id) = agent_config_runtime().await;
    let before = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
    let task = {
        let (runtime, session_id, agent_id) =
            (runtime.clone(), session_id.clone(), agent_id.clone());
        tokio::spawn(
            async move { agent_changes_model(&runtime, &session_id, &agent_id, "opus").await },
        )
    };

    let (card, _, _) = waiting_card(&runtime, &session_id, &agent_id).await;
    runtime
        .resolve_terminal_runtime_interaction(&session_id, &card, "deny", None, Some(OWNER))
        .await
        .unwrap();

    let error = task.await.unwrap().expect_err("denied");
    assert!(
        error.to_string().contains("did not allow this change"),
        "{error}"
    );
    assert_eq!(
        runtime
            .owned
            .agent_store
            .get_agent(&agent_id)
            .unwrap()
            .model(),
        before.model()
    );
}

#[tokio::test]
async fn a_higher_permission_level_takes_a_card_without_allow_always() {
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
    let task = {
        let (runtime, session_id, agent_id) =
            (runtime.clone(), session_id.clone(), agent_id.clone());
        tokio::spawn(async move {
            runtime
                .update_agent_config_as(
                    &session_id,
                    &agent_id,
                    OWNER,
                    None,
                    Some(Some(Yolo)),
                    None,
                    None,
                    None,
                    None,
                    SettingsContext::agent(&agent_id, None),
                )
                .await
                .map(|(agent, _)| agent)
        })
    };

    let (card, choices, _) = waiting_card(&runtime, &session_id, &agent_id).await;
    assert_eq!(choices, ["allow_once", "deny"]);
    runtime
        .resolve_terminal_runtime_interaction(&session_id, &card, "allow_once", None, Some(OWNER))
        .await
        .unwrap();
    let agent = task.await.unwrap().expect("allowed once");
    assert_eq!(agent.permission_level_override(), Some(Yolo));
}

#[tokio::test]
async fn lowering_its_own_permission_level_needs_no_card() {
    use crate::provider::AgentPermissionLevel::{Required, Yolo};
    let (_app, runtime, session_id, agent_id) = agent_config_runtime().await;
    runtime
        .update_agent_config(
            &session_id,
            &agent_id,
            OWNER,
            None,
            Some(Some(Yolo)),
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let agent = runtime
        .update_agent_config_as(
            &session_id,
            &agent_id,
            OWNER,
            None,
            Some(Some(Required)),
            None,
            None,
            None,
            None,
            SettingsContext::agent(&agent_id, None),
        )
        .await
        .map(|(agent, _)| agent)
        .expect("lowering is always allowed");

    assert_eq!(agent.permission_level_override(), Some(Required));
}

#[tokio::test]
async fn an_agent_cannot_widen_what_it_may_change() {
    let (_app, runtime, session_id, agent_id) = agent_config_runtime().await;

    let error = runtime
        .update_agent_config_as(
            &session_id,
            &agent_id,
            OWNER,
            None,
            None,
            None,
            None,
            None,
            Some(AgentSelfSettings {
                models: vec!["*".into()],
                ..Default::default()
            }),
            SettingsContext::agent(&agent_id, None),
        )
        .await
        .expect_err("only the person sets it");

    assert!(error.to_string().contains("only the person"), "{error}");
}

#[tokio::test]
async fn an_agent_above_chooses_a_settings_without_the_set() {
    let (_app, runtime, session_id, agent_id) = agent_config_runtime().await;

    let agent = runtime
        .update_agent_profile_as(
            &session_id,
            &agent_id,
            OWNER,
            None,
            None,
            Some("opus".to_string()),
            None,
            SettingsContext::agent("an-agent-above", None),
        )
        .await
        .map(|(agent, _)| agent)
        .expect("the agents above an agent choose its settings, as when they start it");

    assert_eq!(agent.model(), Some("opus"));
}
