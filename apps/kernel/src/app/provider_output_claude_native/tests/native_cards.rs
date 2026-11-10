//! MP-08 / MP-09 / MP-10 / MP-11: the cards Claude headless asks through its hook: an `AskUserQuestion` call
//! becomes the kernel-owned question card, its answer reaches the waiting
//! hook whatever the agent's permission level, and a stopped turn interrupts it.
use super::*;

#[derive(Clone)]
struct AnsweringBridge {
    interactions: std::sync::Arc<std::sync::Mutex<Vec<RuntimeInteraction>>>,
    answer: crate::provider::ProviderNativeInteractionResolution,
}

impl AnsweringBridge {
    fn answering(choice_id: &str, reply: &str) -> Self {
        Self::resolving("answered", Some(choice_id), Some(reply))
    }

    fn resolving(status: &str, choice_id: Option<&str>, reply: Option<&str>) -> Self {
        Self {
            interactions: Default::default(),
            answer: crate::provider::ProviderNativeInteractionResolution {
                status: status.to_string(),
                choice_id: choice_id.map(str::to_string),
                reply: reply.map(str::to_string),
            },
        }
    }
}

impl ProviderNativeInteractionBridge for AnsweringBridge {
    fn request_blocking(
        &self,
        _session_id: &str,
        interaction: RuntimeInteraction,
    ) -> Result<crate::provider::ProviderNativeInteractionResolution, DaemonError> {
        self.interactions.lock().unwrap().push(interaction);
        Ok(self.answer.clone())
    }
}

struct Fixture {
    root: std::path::PathBuf,
    context_file: String,
    run: RuntimeProviderRun,
}

fn fixture(level: crate::provider::AgentPermissionLevel) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-question-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    fs::write(&context_file, "").expect("context file should be created");
    let context_file = context_file.display().to_string();
    let request = crate::provider::LaunchProviderRequest::new(
        "session-question",
        "claude",
        "claude-headless",
        "default",
        "claude-opus",
    )
    .with_agent_id("agent-question")
    .with_permission_level(level);
    let run = RuntimeProviderRun::new(
        "provider-run-question",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-question".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::from([(
                "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                context_file.clone(),
            )]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    Fixture {
        root,
        context_file,
        run,
    }
}

fn question_event(permission_mode: &str) -> serde_json::Value {
    serde_json::json!({
        "hook_event_name": "PermissionRequest",
        "hook_context_request_id": "request-question",
        "permission_mode": permission_mode,
        "tool_name": "AskUserQuestion",
        "tool_input": {
            "questions": [{
                "question": "Do you prefer red or blue?",
                "header": "Color",
                "multiSelect": false,
                "options": [
                    { "label": "Red", "description": "Choose red." },
                    { "label": "Blue", "description": "Choose blue." }
                ]
            }]
        },
    })
}

fn resolve(fixture: &Fixture, bridge: &AnsweringBridge, event: &serde_json::Value) {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .resolve_permission_event(
            "session-question",
            fixture.run.id(),
            "agent-question",
            &fixture.context_file,
            &fixture.run,
            Some(std::sync::Arc::new(bridge.clone())),
            event,
        )
        .expect("the question should resolve");
}

fn hook_response(fixture: &Fixture) -> serde_json::Value {
    let path = fixture
        .root
        .join("permission-responses/request-question.json");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Ok(raw) = fs::read_to_string(&path) {
            return serde_json::from_str(&raw).expect("the hook response should be JSON");
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the waiting hook never got a response"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn a_question_is_a_choice_card_and_its_answer_reaches_the_hook_at_every_permission_level() {
    for (level, permission_mode) in [
        (crate::provider::AgentPermissionLevel::Required, "default"),
        (crate::provider::AgentPermissionLevel::Yolo, "auto"),
        (
            crate::provider::AgentPermissionLevel::Yolo,
            "bypassPermissions",
        ),
    ] {
        let fixture = fixture(level);
        let event = question_event(permission_mode);
        assert!(
            should_bridge_claude_permission(&event),
            "{permission_mode}: a question always needs a person"
        );
        let bridge = AnsweringBridge::answering("option-1", "Blue");

        resolve(&fixture, &bridge, &event);

        let response = hook_response(&fixture);
        let interactions = bridge.interactions.lock().unwrap();
        assert_eq!(interactions.len(), 1, "{permission_mode}");
        let card = &interactions[0];
        assert_eq!(card.kind(), RuntimeInteractionKind::Choice);
        assert_eq!(card.title(), Some("Color"));
        assert!(card.message().contains("Do you prefer red or blue?"));
        let labels = card
            .choices()
            .iter()
            .map(|choice| choice.label())
            .collect::<Vec<_>>();
        assert_eq!(labels, ["Red", "Blue"], "{permission_mode}");
        assert!(
            card.custom_choice().is_some(),
            "{permission_mode}: Claude offers a typed answer too"
        );
        assert_eq!(response["behavior"], "allow", "{permission_mode}");
        assert_eq!(
            response["updatedInput"]["answers"]["Do you prefer red or blue?"], "Blue",
            "{permission_mode}"
        );
        assert_eq!(
            response["updatedInput"]["questions"], event["tool_input"]["questions"],
            "{permission_mode}: the questions stay as Claude asked them"
        );
        let _ = fs::remove_dir_all(&fixture.root);
    }
}

#[test]
fn a_question_withdrawn_with_its_stopped_turn_interrupts_claude() {
    let fixture = fixture(crate::provider::AgentPermissionLevel::Required);
    let bridge = AnsweringBridge::resolving("timed_out", None, None);

    resolve(&fixture, &bridge, &question_event("default"));

    let response = hook_response(&fixture);
    assert_eq!(response["behavior"], "deny");
    assert_eq!(
        response["interrupt"], true,
        "Claude must end the turn, not ask again: {response}"
    );
    let _ = fs::remove_dir_all(&fixture.root);
}

#[test]
fn a_card_is_withdrawn_only_when_it_closed_unanswered_before_its_deadline() {
    let timed_out = crate::provider::ProviderNativeInteractionResolution {
        status: "timed_out".to_string(),
        choice_id: None,
        reply: None,
    };
    let deadline =
        std::time::Duration::from_secs(crate::provider::CLAUDE_NATIVE_PERMISSION_TIMEOUT_SECS);
    let asked_just_now = std::time::Instant::now();
    let asked_a_full_wait_ago = std::time::Instant::now() - deadline;
    assert!(claude_card_withdrawn(&timed_out, asked_just_now));
    assert!(
        !claude_card_withdrawn(&timed_out, asked_a_full_wait_ago),
        "a card nobody answered in time is a timeout, not a stop"
    );
    let answered = crate::provider::ProviderNativeInteractionResolution {
        status: "answered".to_string(),
        choice_id: Some("option-0".to_string()),
        reply: Some("Red".to_string()),
    };
    assert!(!claude_card_withdrawn(&answered, asked_just_now));
}

#[test]
fn a_tool_approval_withdrawn_with_its_stopped_turn_interrupts_claude() {
    let fixture = fixture(crate::provider::AgentPermissionLevel::Required);
    let bridge = AnsweringBridge::resolving("timed_out", None, None);
    let event = serde_json::json!({
        "hook_event_name": "PermissionRequest",
        "hook_context_request_id": "request-question",
        "permission_mode": "default",
        "tool_name": "Bash",
        "tool_input": { "command": "sleep 20" },
    });

    resolve(&fixture, &bridge, &event);

    let response = hook_response(&fixture);
    assert_eq!(response["behavior"], "deny");
    assert_eq!(
        response["interrupt"], true,
        "Stop must end the turn instead of letting Claude retry the call: {response}"
    );
    let _ = fs::remove_dir_all(&fixture.root);
}
