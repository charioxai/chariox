use super::*;
use crate::local::{PaneCardChoice, PaneCardReply, PaneIdentity, PaneSettings};

fn card(text_terminal: bool) -> PaneCard {
    PaneCard {
        id: "card".into(),
        asker_name: Some("helper".into()),
        title: Some("Deploy".into()),
        message: "Ship it?".into(),
        choices: vec![
            PaneCardChoice {
                id: "yes".into(),
                label: "Yes".into(),
            },
            PaneCardChoice {
                id: "no".into(),
                label: "No".into(),
            },
        ],
        reply: Some(PaneCardReply {
            id: "other".into(),
            label: "Something else".into(),
            placeholder: None,
            secret: false,
        }),
        text_terminal,
        requested_at_ms: 1,
        open_elsewhere: "Open in Chariox: https://chariox.test/terminal?session=s&agent=a".into(),
    }
}

fn feed(cards: Vec<PaneCard>, link: Option<&str>) -> PaneFeed {
    PaneFeed {
        title: "Room · claude-1".into(),
        identity: PaneIdentity {
            room_id: "s".into(),
            room_name: "Room".into(),
            agent_id: "a".into(),
            agent_name: "claude-1".into(),
            parents: vec!["lead".into()],
            project_id: "p".into(),
            placement: None,
        },
        settings: PaneSettings {
            provider: "claude".into(),
            account: Some("Work".into()),
            model: Some("claude-haiku-5-5".into()),
            effort: None,
            execution_mode: AgentExecutionMode::Build,
            permission_level: AgentPermissionLevel::Yolo,
        },
        state: if cards.is_empty() {
            PaneState::Idle
        } else {
            PaneState::WaitingForAnswer
        },
        goal: None,
        cards,
        stripes: Vec::new(),
        notices: Vec::new(),
        link: link.map(str::to_owned),
    }
}

fn dialog(answers: DialogAnswers) -> Dialog {
    Dialog {
        run_id: "run".into(),
        session_id: "s".into(),
        owner_user_id: "owner".into(),
        card_id: "card".into(),
        answers,
    }
}

#[test]
fn the_status_line_names_the_pane_its_settings_and_what_waits() {
    let idle = feed(Vec::new(), None);
    assert_eq!(
        status_lines(&idle),
        "Room · lead ▸ claude-1 · Work · claude-haiku-5-5 · default · build · Full access"
    );
    let waiting = feed(vec![card(true)], None);
    assert_eq!(
        status_lines(&waiting).lines().nth(1),
        Some("Waiting for your answer · 1 card waiting · Open in Chariox: https://chariox.test/terminal?session=s&agent=a")
    );
}

#[test]
fn a_card_a_text_terminal_answers_is_a_form_and_others_are_the_pane_link() {
    let form = feed(vec![card(true)], Some("https://chariox.test/terminal"));
    let (request, (_, answers)) = dialog_for(&form.cards[0], &form).expect("a form");
    assert_eq!(request["mode"], "form");
    assert_eq!(request["message"], "Deploy\n\nhelper: Ship it?");
    assert_eq!(
        request["requestedSchema"]["required"],
        serde_json::json!([])
    );
    assert!(matches!(answers, DialogAnswers::Form { .. }));

    let link = feed(vec![card(false)], Some("https://chariox.test/terminal"));
    let (request, (_, answers)) = dialog_for(&link.cards[0], &link).expect("a link");
    assert_eq!(request["mode"], "url");
    assert_eq!(request["url"], "https://chariox.test/terminal");
    assert_eq!(answers, DialogAnswers::Link);

    let unpaired = feed(vec![card(false)], None);
    assert!(
        dialog_for(&unpaired.cards[0], &unpaired).is_none(),
        "with no link the status line names the Room and agent"
    );
}

#[test]
fn only_an_accepted_known_choice_or_a_typed_reply_answers_the_card() {
    let form = dialog(DialogAnswers::Form {
        choices: vec!["yes".into(), "no".into()],
        reply_choice: Some("other".into()),
    });
    let accept = |content: Value| {
        answer_for(
            &form,
            &serde_json::json!({ "action": "accept", "content": content }),
        )
    };
    let answer = accept(serde_json::json!({ "choice": "no" })).expect("a choice");
    assert_eq!(
        (answer.choice_id.as_str(), answer.custom_reply),
        ("no", None)
    );
    let answer = accept(serde_json::json!({ "choice": "yes", "reply": "later" })).expect("a reply");
    assert_eq!(
        (answer.choice_id.as_str(), answer.custom_reply.as_deref()),
        ("other", Some("later"))
    );
    assert!(accept(serde_json::json!({ "choice": "maybe" })).is_none());
    assert!(answer_for(&form, &serde_json::json!({ "action": "decline" })).is_none());
    assert!(answer_for(&form, &serde_json::json!({ "action": "cancel" })).is_none());
    assert!(answer_for(
        &dialog(DialogAnswers::Link),
        &serde_json::json!({ "action": "accept" })
    )
    .is_none());
}

#[test]
fn claudes_permission_modes_map_to_the_kernels_mode_and_permissions() {
    assert_eq!(scope_for("plan"), (Some(AgentExecutionMode::Plan), None));
    assert_eq!(
        scope_for("default"),
        (
            Some(AgentExecutionMode::Build),
            Some(AgentPermissionLevel::Required)
        )
    );
    assert_eq!(
        scope_for("bypassPermissions"),
        (
            Some(AgentExecutionMode::Build),
            Some(AgentPermissionLevel::Yolo)
        )
    );
    assert_eq!(
        scope_for("acceptEdits"),
        (Some(AgentExecutionMode::Build), None)
    );
    assert_eq!(scope_for("unknown"), (None, None));
}
