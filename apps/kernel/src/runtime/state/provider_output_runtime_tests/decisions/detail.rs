//! What a parent reads of a card it holds: the title as `question`, and the card's own text as
//! `detail`, because a command card's title ("Allow Claude to run a command?") does not say
//! which command it allows.
use super::*;

fn command_card(id: &str, agent: &str, message: &str) -> RuntimeInteraction {
    RuntimeInteraction::new(
        id,
        agent,
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        Some("Allow Claude to run a command?".into()),
        message,
        vec![
            RuntimeInteractionChoice::new("allow", "Allow", "allow", None),
            RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
        ],
        None,
        None,
        None,
    )
}

/// The payload the holder reads through its inbox tool, for the one `child_decision` event.
fn read_payload(t: &Tree, holder: &str) -> serde_json::Value {
    let read = t
        .runtime
        .read_agent_inbox(&t.room, holder, &serde_json::json!({}))
        .unwrap();
    let events = read["events"].as_array().unwrap();
    let decision: Vec<_> = events
        .iter()
        .filter(|event| event["kind"] == "child_decision")
        .collect();
    assert_eq!(decision.len(), 1, "{read}");
    decision[0]["payload"].clone()
}

#[tokio::test]
async fn a_command_card_tells_the_parent_which_command_it_allows() {
    let t = tree(
        "decision-detail",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    t.raise(command_card(
        "perm-detail",
        &t.c,
        "Command:\n\ncargo test -p chariox-kernel decision_holder",
    ))
    .await;
    let payload = read_payload(&t, &t.p);
    assert_eq!(payload["question"], "Allow Claude to run a command?");
    assert_eq!(
        payload["detail"],
        "Command:\n\ncargo test -p chariox-kernel decision_holder"
    );
}

#[tokio::test]
async fn a_long_command_is_cut_at_the_detail_limit_and_says_so() {
    let t = tree(
        "decision-detail-long",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    t.raise(command_card(
        "perm-detail-long",
        &t.c,
        &format!("Command:\n\n{}", "x".repeat(5000)),
    ))
    .await;
    let detail = read_payload(&t, &t.p)["detail"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(detail.chars().count(), 1000 + 1, "the limit and the mark");
    assert!(detail.ends_with('…'), "a cut command must look cut");
}

// A cut can hide the part that matters (`... ; rm -rf ~`), and a closing mark in the text is not a
// field a holder checks, so the payload says it.
#[tokio::test]
async fn a_cut_detail_is_flagged_in_the_payload_and_a_whole_one_is_not() {
    let long = tree(
        "decision-detail-cut-flag",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    long.raise(command_card(
        "perm-detail-cut-flag",
        &long.c,
        &format!("Command:\n\n{} ; rm -rf ~", "x".repeat(2000)),
    ))
    .await;
    let cut = read_payload(&long, &long.p);
    assert_eq!(cut["detail_truncated"], true, "{cut}");
    assert!(!cut["detail"].as_str().unwrap().contains("rm -rf"), "{cut}");

    let whole = tree(
        "decision-detail-whole-flag",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    whole
        .raise(command_card(
            "perm-detail-whole-flag",
            &whole.c,
            &format!("Command:\n\n{}", "x".repeat(990)),
        ))
        .await;
    let payload = read_payload(&whole, &whole.p);
    assert!(payload.get("detail_truncated").is_none(), "{payload}");
}

// The flag only helps a holder that is told what to do with it.
#[test]
fn the_answer_help_says_what_to_do_with_a_cut_detail() {
    let docs = crate::runtime::metaagent_command_registry::command_docs(
        crate::transport::runtime_tools::MetaCommandDocsArgs {
            command: "agent answer".into(),
        },
    )
    .unwrap();
    let description = docs["description"].as_str().unwrap();
    assert!(description.contains("detail_truncated"), "{description}");
    assert!(description.contains("agent pass"), "{description}");
}

#[tokio::test]
async fn a_secret_shaped_value_in_a_command_is_not_passed_on() {
    let t = tree(
        "decision-detail-secret",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let token = format!("ghp_{}", "a1B2c3D4e5".repeat(4));
    t.raise(command_card(
        "perm-detail-token",
        &t.c,
        &format!("Command:\n\ncurl -H 'Authorization: Bearer {token}' https://example.test"),
    ))
    .await;
    let payload = read_payload(&t, &t.p);
    assert!(!payload.to_string().contains(&token), "{payload}");
    assert!(payload["detail"].as_str().unwrap().contains("curl"));
}

#[tokio::test]
async fn a_titled_card_reads_its_text_as_the_detail() {
    let t = tree(
        "decision-detail-titled",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    t.raise(permission("perm-detail-titled", &t.c)).await;
    let payload = read_payload(&t, &t.p);
    assert_eq!(payload["question"], "Allow command?");
    assert_eq!(payload["detail"], "Run `ls`?");
}

// An agent has one question open at a time, so the untitled card gets a tree of its own.
#[tokio::test]
async fn a_card_with_no_text_beyond_its_title_has_no_detail() {
    let t = tree(
        "decision-detail-none",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let untitled = RuntimeInteraction::new(
        "ask-detail-none",
        &t.c,
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        None,
        "Run ls?",
        vec![RuntimeInteractionChoice::new(
            "allow", "Allow", "allow", None,
        )],
        None,
        None,
        None,
    );
    t.raise(untitled).await;
    let payload = read_payload(&t, &t.p);
    assert_eq!(payload["question"], "Run ls?");
    assert!(
        payload.get("detail").is_none(),
        "the message is the question already: {payload}"
    );
}

// A decision only the user may take is never a `child_decision`, so its text reaches no parent.
#[tokio::test]
async fn a_card_only_the_user_may_answer_reaches_no_parent_with_its_text() {
    let t = tree(
        "decision-detail-user-only",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let secret = RuntimeInteraction::new(
        "sec-detail",
        &t.c,
        RuntimeInteractionKind::Choice,
        RuntimeInteractionLevel::Info,
        Some("Enter the deploy key".into()),
        "Type the deploy key for the staging host",
        Vec::new(),
        Some(RuntimeInteractionCustomChoice::secret(
            "secret",
            "Key",
            None,
            Some(1),
            Some(64),
        )),
        None,
        None,
    );
    t.raise(secret).await;
    for agent in [&t.p, &t.g] {
        assert!(t.decision_events(agent).is_empty(), "{agent} was told");
    }
}

// An event stored before `detail` existed is read as it was.
#[tokio::test]
async fn an_event_stored_without_detail_still_reads() {
    let t = tree(
        "decision-detail-old",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    t.runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Occur(
            crate::durable_state::agent_lifecycle::occurrence(
                &t.room,
                &t.p,
                &t.c,
                "child-decision-old",
                "child_decision",
                serde_json::json!({
                    "interaction_id": "perm-old",
                    "child_agent_id": t.c,
                    "child_alias": "c",
                    "question": "Allow command?",
                    "choices": [{"id": "allow", "label": "Allow"}],
                    "text_allowed": false,
                    "needs_answer": true,
                }),
            ),
        ))
        .unwrap();
    let payload = read_payload(&t, &t.p);
    assert_eq!(payload["question"], "Allow command?");
    assert!(payload.get("detail").is_none(), "{payload}");
}
