//! An external agent's access grant answers the person's cards for them, through a real kernel
//! and a real grant holder process, while secret entry, passkeys and payments stay the person's.
use super::waiter::file_text;
use super::*;

fn answer(holder: &mut Client, id: &str, choice: &str) -> Value {
    holder.request(serde_json::json!({"RespondToInteraction":{
        "session_id":SESSION, "interaction_id":id, "choice_id":choice
    }}))
}

/// What `chariox interaction answer` sends for a listed card: the Room's answer, or the answer
/// to a card outside any Room.
fn settle(holder: &mut Client, card: &Value, choice: &str) -> Value {
    let id = &card["interaction_id"];
    holder.request(match card["session_id"].as_str() {
        Some(session) => serde_json::json!({"RespondToInteraction":{
            "session_id":session, "interaction_id":id, "choice_id":choice
        }}),
        None => serde_json::json!({"AnswerUserDomainInteraction":{
            "interaction_id":id, "choice_id":choice
        }}),
    })
}

#[tokio::test]
async fn a_grant_holder_answers_the_persons_cards_but_not_secrets_passkeys_or_payments() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    grant(&mut kernel, &mut holder).await;
    kernel.control("owner-cards").await;

    let listed = holder.request(serde_json::json!({"ListOwnerInteractions":{}}));
    let cards = listed["response"]["OwnerInteractionsListed"]["interactions"]
        .as_array()
        .unwrap_or_else(|| panic!("{listed}"))
        .clone();
    let person_only = |id: &str| {
        cards
            .iter()
            .find(|card| card["interaction_id"] == id)
            .unwrap_or_else(|| panic!("{id} not listed: {listed}"))["person_only"]
            .clone()
    };
    // Every card listed as answerable is one the grant holder settles: in a Room, from a sub-agent
    // two levels down, an install, and an App's host offer outside any Room.
    let answerable = [
        ("capability-request-card", "allow"),
        ("machine-acquisition-card", "approve"),
        ("app-install-card", "approve"),
        ("sub-agent-question", "green"),
        ("app_host_detached", "decline"),
    ];
    let listed_answerable = cards
        .iter()
        .filter(|card| card["person_only"].is_null())
        .map(|card| card["interaction_id"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        listed_answerable,
        answerable.iter().map(|(id, _)| *id).collect(),
        "{listed}"
    );
    let detached = cards
        .iter()
        .find(|card| card["interaction_id"] == "app_host_detached")
        .unwrap();
    assert!(detached.get("session_id").is_none(), "{detached}");
    for (id, reason) in [
        ("vault-card-payment", "it approves a payment"),
        ("secret-card", "it asks for a secret"),
        ("critical-test", "it needs the passkey"),
    ] {
        assert_eq!(person_only(id), reason, "{listed}");
    }

    let attachment = holder.attach();
    for (id, choice) in answerable {
        let card = cards
            .iter()
            .find(|card| card["interaction_id"] == id)
            .unwrap();
        let result = settle(&mut holder, card, choice);
        assert!(result["error"].is_null(), "{id}: {result}");
        assert_eq!(
            serde_json::from_str::<Value>(
                &file_text(kernel.root.join(format!("answered-{id}"))).await
            )
            .unwrap(),
            choice
        );
    }
    // Every client of the Room is told who answered for the person.
    let notices = holder.request(serde_json::json!({"PollRuntimeNotices":{
        "session_id":SESSION, "attachment_id":attachment
    }}));
    assert!(
        notices
            .to_string()
            .contains("answered \\\"Chariox resource access\\\" on the owner's behalf: Allow."),
        "{notices}"
    );

    // Payments, secrets and passkeys stay open for the person, for every choice.
    for id in ["vault-card-payment", "secret-card", "critical-test"] {
        let refused = answer(&mut holder, id, "deny");
        assert_eq!(
            refused["error"]["code"], "kernel_access_human_only",
            "{id}: {refused}"
        );
    }
    let open = holder.request(serde_json::json!({"ListOwnerInteractions":{}}));
    let open = open["response"]["OwnerInteractionsListed"]["interactions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|card| card["interaction_id"].as_str().unwrap().to_owned())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        open,
        ["critical-test", "secret-card", "vault-card-payment"]
            .map(str::to_owned)
            .into()
    );
}
