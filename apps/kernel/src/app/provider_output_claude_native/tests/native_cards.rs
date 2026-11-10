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
    static FIXTURES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-question-test-{}-{}-{}",
        std::process::id(),
        timestamp_millis(),
        FIXTURES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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
    let wait =
        std::time::Duration::from_secs(crate::provider::CLAUDE_NATIVE_PERMISSION_TIMEOUT_SECS);
    let open_for_a_full_wait = std::time::Instant::now() + wait;
    let due_now = std::time::Instant::now();
    assert!(claude_card_withdrawn(&timed_out, open_for_a_full_wait));
    assert!(
        !claude_card_withdrawn(&timed_out, due_now),
        "a card nobody answered in time is a timeout, not a stop"
    );
    let answered = crate::provider::ProviderNativeInteractionResolution {
        status: "answered".to_string(),
        choice_id: Some("option-0".to_string()),
        reply: Some("Red".to_string()),
    };
    assert!(!claude_card_withdrawn(&answered, open_for_a_full_wait));
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

/// Answers each card with the next scripted choice, after a pause.
#[derive(Clone)]
struct ScriptedBridge {
    interactions: std::sync::Arc<std::sync::Mutex<Vec<RuntimeInteraction>>>,
    answers: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<(String, String)>>>,
    pause: std::time::Duration,
}

impl ScriptedBridge {
    fn new(answers: &[(&str, &str)], pause: std::time::Duration) -> Self {
        Self {
            interactions: Default::default(),
            answers: std::sync::Arc::new(std::sync::Mutex::new(
                answers
                    .iter()
                    .map(|(id, reply)| (id.to_string(), reply.to_string()))
                    .collect(),
            )),
            pause,
        }
    }
}

impl ProviderNativeInteractionBridge for ScriptedBridge {
    fn request_blocking(
        &self,
        _session_id: &str,
        interaction: RuntimeInteraction,
    ) -> Result<crate::provider::ProviderNativeInteractionResolution, DaemonError> {
        self.interactions.lock().unwrap().push(interaction);
        std::thread::sleep(self.pause);
        let (choice_id, reply) = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("a card nobody scripted an answer for");
        Ok(crate::provider::ProviderNativeInteractionResolution {
            status: "answered".to_string(),
            choice_id: Some(choice_id),
            reply: Some(reply),
        })
    }
}

fn resolve_scripted(fixture: &Fixture, bridge: &ScriptedBridge, event: &serde_json::Value) {
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

fn labels(card: &RuntimeInteraction) -> Vec<&str> {
    card.choices().iter().map(|choice| choice.label()).collect()
}

#[test]
fn a_multi_select_question_collects_every_choice_before_it_answers() {
    let fixture = fixture(crate::provider::AgentPermissionLevel::Required);
    let event = serde_json::json!({
        "hook_event_name": "PermissionRequest",
        "hook_context_request_id": "request-question",
        "permission_mode": "default",
        "tool_name": "AskUserQuestion",
        "tool_input": {
            "questions": [{
                "question": "Which colors do you like?",
                "header": "Colors",
                "multiSelect": true,
                "options": [{ "label": "Red" }, { "label": "Blue" }, { "label": "Green" }]
            }]
        },
    });
    let bridge = ScriptedBridge::new(
        &[("option-0", "Red"), ("option-2", "Green"), ("done", "Done")],
        std::time::Duration::ZERO,
    );

    resolve_scripted(&fixture, &bridge, &event);

    let response = hook_response(&fixture);
    let cards = bridge.interactions.lock().unwrap();
    assert_eq!(cards.len(), 3, "one card per choice, then Done");
    assert_eq!(labels(&cards[0]), ["Red", "Blue", "Green"]);
    assert_eq!(labels(&cards[1]), ["Blue", "Green", "Done"]);
    assert_eq!(labels(&cards[2]), ["Blue", "Done"]);
    assert_eq!(response["behavior"], "allow");
    assert_eq!(
        response["updatedInput"]["answers"]["Which colors do you like?"],
        "Red, Green"
    );
    let _ = fs::remove_dir_all(&fixture.root);
}

#[test]
fn the_questions_of_one_call_share_one_deadline() {
    let fixture = fixture(crate::provider::AgentPermissionLevel::Required);
    let mut event = question_event("default");
    event["tool_input"]["questions"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "question": "Large or small?",
            "header": "Size",
            "multiSelect": false,
            "options": [{ "label": "Large" }, { "label": "Small" }]
        }));
    let bridge = ScriptedBridge::new(
        &[("option-0", "Red"), ("option-1", "Small")],
        std::time::Duration::from_millis(1_100),
    );

    resolve_scripted(&fixture, &bridge, &event);

    let response = hook_response(&fixture);
    let cards = bridge.interactions.lock().unwrap();
    assert_eq!(cards.len(), 2);
    assert_eq!(
        cards[0].timeout_sec(),
        Some(crate::provider::CLAUDE_NATIVE_PERMISSION_TIMEOUT_SECS)
    );
    assert!(
        cards[1].timeout_sec().unwrap() < crate::provider::CLAUDE_NATIVE_PERMISSION_TIMEOUT_SECS,
        "the second card gets only what the first left of the hook's wait: {:?}",
        cards[1].timeout_sec()
    );
    assert_eq!(
        response["updatedInput"]["answers"]["Large or small?"],
        "Small"
    );
    let _ = fs::remove_dir_all(&fixture.root);
}
