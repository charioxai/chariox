//! The Room has one Vault unlock card. An agent that needs the Vault while another agent's card
//! is open joins it and is told nothing more, however often it retries.
use super::child_finished::family;
use super::*;

fn vault_notices(runtime: &KernelRuntimeState, room: &str, agent: &str) -> usize {
    runtime
        .owned
        .operational_history_store
        .load_session_history_entries(room, Some(agent))
        .unwrap()
        .into_iter()
        .filter(|entry| entry.text.contains("Vault locked: unlock to start"))
        .count()
}

#[tokio::test]
async fn an_agent_joining_another_agents_open_vault_card_adds_no_notice() {
    let team = family("vault-notice-join", 2, None, true).await;
    let (opener, joiner) = (&team.kids[0].child, &team.kids[1].child);
    let vault_path = std::env::temp_dir().join(format!("vault-notice-join-{}", std::process::id()));

    // No card is open: the start says at once that it waits on the Vault.
    team.runtime
        .say_launch_waits_on_vault(&team.room, joiner, "work", &vault_path);
    assert_eq!(vault_notices(&team.runtime, &team.room, joiner), 1);

    // The other agent opens the Room's unlock card.
    let card = crate::session::RuntimeInteraction::new(
        "vault-unlock-room",
        opener.clone(),
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Info,
        Some("Unlock Chariox Vault".into()),
        "Unlock the Vault",
        vec![crate::session::RuntimeInteractionChoice::new(
            "a", "A", "a", None,
        )],
        None,
        Some(280),
        None,
    );
    let _pending = team
        .runtime
        .create_runtime_interaction(
            crate::runtime::state::CardKind::VaultCredentialPayment,
            &team.room,
            card,
        )
        .await
        .expect("the card opens");

    for _ in 0..3 {
        team.runtime
            .say_launch_waits_on_vault(&team.room, joiner, "work", &vault_path);
    }
    assert_eq!(
        vault_notices(&team.runtime, &team.room, joiner),
        1,
        "joining an open card adds no notice"
    );
}
