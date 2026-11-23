//! Row 274: ONE card on the owner's computer for a Claude login that only exists natively.
use super::*;

#[tokio::test]
async fn a_native_only_claude_login_raises_one_card_and_cancel_starts_nothing() {
    let f = fixture("claude-login-card", false).await;
    let parent = f.runtime.owned.agent_store.get_agent(&f.agent).unwrap();
    let request =
        crate::agent::CreateAgentRequest::new(f.room.clone(), "claude").with_parent(&parent);
    let id = "claude-login-for-machines:work";
    let cards = |runtime: &KernelRuntimeState| {
        runtime
            .owned
            .session_store
            .get_session(&f.room)
            .unwrap()
            .active_interactions()
            .iter()
            .filter(|card| card.id() == id)
            .count()
    };

    assert!(
        f.runtime
            .raise_setup_token_card(&request, "work", "Work")
            .await
    );
    assert_eq!(cards(&f.runtime), 1);
    // The agent spawns again before the owner answered: still ONE card.
    assert!(
        f.runtime
            .raise_setup_token_card(&request, "work", "Work")
            .await
    );
    assert_eq!(cards(&f.runtime), 1);

    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    let card = session
        .active_interactions()
        .iter()
        .find(|card| card.id() == id)
        .unwrap();
    let json = serde_json::to_string(card).unwrap();
    assert!(json.contains("Create a Claude login your other machines can use"));
    assert!(json.contains("Create login"));
    assert!(!json.contains("on that machine"));

    f.runtime
        .resolve_runtime_interaction(&f.room, id, "cancel", None)
        .await
        .unwrap();
    for _ in 0..50 {
        if cards(&f.runtime) == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(cards(&f.runtime), 0);
    // A spawn with no agent has no card to raise.
    let alone = crate::agent::CreateAgentRequest::new(f.room.clone(), "claude");
    assert!(
        !f.runtime
            .raise_setup_token_card(&alone, "work", "Work")
            .await
    );
}

#[tokio::test]
async fn a_login_that_cannot_start_after_create_login_is_reported_to_the_agent() {
    let f = fixture("claude-login-card-failure", false).await;
    let parent = f.runtime.owned.agent_store.get_agent(&f.agent).unwrap();
    let request =
        crate::agent::CreateAgentRequest::new(f.room.clone(), "claude").with_parent(&parent);
    // The profile named on the card does not exist, so the sign-in cannot start.
    let id = "claude-login-for-machines:vanished";
    assert!(
        f.runtime
            .raise_setup_token_card(&request, "vanished", "Vanished")
            .await
    );
    f.runtime
        .resolve_runtime_interaction(&f.room, id, "create", None)
        .await
        .unwrap();
    for _ in 0..100 {
        if f.notices().contains("could not start") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let notices = f.notices();
    assert!(
        notices.contains("Claude sign-in could not start for vanished"),
        "{notices}"
    );
    // The recovery is the card on the next spawn, which is raised again.
    assert!(
        notices.contains("spawn the agent again to get the card back"),
        "{notices}"
    );
    assert!(
        f.runtime
            .raise_setup_token_card(&request, "vanished", "Vanished")
            .await
    );
}
