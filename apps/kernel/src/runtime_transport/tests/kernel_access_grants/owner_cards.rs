//! An external agent's access grant answers the person's cards for them, through a real kernel
//! and a real grant holder process, while secret entry, passkeys and payments stay the person's.
use super::waiter::file_text;
use super::*;

fn answer(holder: &mut Client, id: &str, choice: &str) -> Value {
    holder.request(serde_json::json!({"RespondToInteraction":{
        "session_id":SESSION, "interaction_id":id, "choice_id":choice
    }}))
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
    for id in [
        "capability-request-card",
        "machine-acquisition-card",
        "app-install-card",
    ] {
        assert!(person_only(id).is_null(), "{id}: {listed}");
    }
    for (id, reason) in [
        ("vault-card-payment", "it approves a payment"),
        ("secret-card", "it asks for a secret"),
        ("critical-test", "it needs the passkey"),
    ] {
        assert_eq!(person_only(id), reason, "{listed}");
    }

    let attachment = holder.attach();
    // A sub-agent's resource request, a machine and an install: each answer reaches the asker.
    for (id, choice) in [
        ("capability-request-card", "allow"),
        ("machine-acquisition-card", "approve"),
        ("app-install-card", "approve"),
    ] {
        let result = answer(&mut holder, id, choice);
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
