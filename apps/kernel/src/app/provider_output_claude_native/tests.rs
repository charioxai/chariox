mod account_usage;
mod context_failure;
mod native_permission_race;
mod prepared_context;

use super::transcript::ClaudeTranscriptCursor;
use super::*;

#[derive(Clone, Default)]
struct RecordingPermissionBridge {
    interaction_ids: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    messages: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl ProviderNativeInteractionBridge for RecordingPermissionBridge {
    fn request_blocking(
        &self,
        _session_id: &str,
        interaction: RuntimeInteraction,
    ) -> Result<crate::provider::ProviderNativeInteractionResolution, DaemonError> {
        self.messages
            .lock()
            .unwrap()
            .push(interaction.message().to_string());
        self.interaction_ids
            .lock()
            .expect("permission interaction recorder should not be poisoned")
            .push(interaction.id().to_string());
        Ok(crate::provider::ProviderNativeInteractionResolution {
            status: "answered".to_string(),
            choice_id: Some("allow_once".to_string()),
            reply: Some("allow".to_string()),
        })
    }
}

#[derive(Clone)]
pub(crate) struct StartupTrustBridge {
    interactions: std::sync::Arc<std::sync::Mutex<Vec<RuntimeInteraction>>>,
    resolution: std::sync::Arc<(
        std::sync::Mutex<Option<crate::provider::ProviderNativeInteractionResolution>>,
        std::sync::Condvar,
    )>,
}

impl Default for StartupTrustBridge {
    fn default() -> Self {
        Self {
            interactions: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            resolution: std::sync::Arc::new((
                std::sync::Mutex::new(None),
                std::sync::Condvar::new(),
            )),
        }
    }
}

impl StartupTrustBridge {
    fn wait_for_interaction(&self) -> RuntimeInteraction {
        self.wait_for_interaction_with_pump(|| {})
    }

    pub(crate) fn wait_for_interaction_with_pump(
        &self,
        mut pump: impl FnMut(),
    ) -> RuntimeInteraction {
        for _ in 0..500 {
            pump();
            if let Some(interaction) = self
                .interactions
                .lock()
                .expect("startup trust interactions should not be poisoned")
                .first()
                .cloned()
            {
                return interaction;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("workspace trust interaction was not projected");
    }

    fn resolve(&self, choice_id: &str, reply: &str) {
        let (lock, wake) = &*self.resolution;
        *lock
            .lock()
            .expect("startup trust resolution should not be poisoned") =
            Some(crate::provider::ProviderNativeInteractionResolution {
                status: "answered".to_string(),
                choice_id: Some(choice_id.to_string()),
                reply: Some(reply.to_string()),
            });
        wake.notify_all();
    }

    pub(crate) fn resolve_default_no(&self) {
        let (lock, wake) = &*self.resolution;
        *lock
            .lock()
            .expect("startup trust resolution should not be poisoned") =
            Some(crate::provider::ProviderNativeInteractionResolution {
                status: "timed_out".to_string(),
                choice_id: Some("deny".to_string()),
                reply: Some("deny".to_string()),
            });
        wake.notify_all();
    }
}

impl ProviderNativeInteractionBridge for StartupTrustBridge {
    fn request_blocking(
        &self,
        _session_id: &str,
        interaction: RuntimeInteraction,
    ) -> Result<crate::provider::ProviderNativeInteractionResolution, DaemonError> {
        self.interactions
            .lock()
            .expect("startup trust interactions should not be poisoned")
            .push(interaction);
        let (lock, wake) = &*self.resolution;
        let mut resolution = lock
            .lock()
            .expect("startup trust resolution should not be poisoned");
        while resolution.is_none() {
            resolution = wake
                .wait(resolution)
                .expect("startup trust resolution wait should not be poisoned");
        }
        Ok(resolution
            .take()
            .expect("startup trust resolution should be available"))
    }
}

pub(crate) fn startup_readiness_run(
    session_id: &str,
    agent_id: &str,
    provider_run_id: &str,
    context_file: &std::path::Path,
    events_file: &std::path::Path,
    pty_command: String,
) -> RuntimeProviderRun {
    let request = crate::provider::LaunchProviderRequest::new(
        session_id,
        "claude",
        "claude-headless",
        "default",
        "claude-opus",
    )
    .with_agent_id(agent_id)
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    RuntimeProviderRun::new(
        provider_run_id,
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: format!("{provider_run_id}-process"),
            pty_target: Some(provider_run_id.to_string()),
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec!["-lc".to_string(), pty_command],
            pty_env: std::collections::BTreeMap::from([
                (
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                    context_file.display().to_string(),
                ),
                (
                    "CHARIOX_CLAUDE_NATIVE_EVENTS".to_string(),
                    events_file.display().to_string(),
                ),
            ]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    )
}

#[test]
fn repeated_claude_permission_render_is_stored_once() {
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-permission-recent-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("context.json");
    let context_file = context_file.display().to_string();
    let rendered =
        "--dangerously-skip-permissions cannot be used with root/sudo privileges for security reasons";

    let first = update_claude_permission_recent(&context_file, rendered);
    let second = update_claude_permission_recent(&context_file, rendered);

    assert_eq!(first, second);
    assert_eq!(second.matches(rendered).count(), 1, "{second}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn claude_permission_detection_bridges_every_permission_request() {
    let mcp_permission = serde_json::json!({
        "hook_event_name": "PermissionRequest",
        "hook_context_request_id": "request-mcp",
        "tool_name": "mcp__native_test__echo_marker",
        "tool_input": { "marker": "native-skill-marker" },
    });
    assert!(should_bridge_claude_permission(&mcp_permission));
    assert!(claude_rendered_permission_visible(
        "Tool use native-test - echo_marker(marker: native-skill-marker) (MCP)\n\
         Do you want to proceed?\n1. Yes\n2. Yes, and don't ask again\n3. No",
    ));

    assert!(should_bridge_claude_permission(&serde_json::json!({
        "hook_event_name": "PermissionRequest",
        "tool_name": "mcp__chariox__session_status",
    })));
    assert!(!should_bridge_claude_permission(&serde_json::json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "mcp__chariox__session_status",
    })));
    assert!(!should_bridge_claude_permission(&serde_json::json!({
        "hook_event_name": "PreToolUse",
        "permission_mode": "bypassPermissions",
        "tool_name": "Bash",
    })));
    assert!(!should_bridge_claude_permission(&serde_json::json!({
        "hook_event_name": "PermissionRequest",
        "permission_mode": "bypassPermissions",
        "tool_name": "Bash",
    })));
}

#[test]
fn yolo_rendered_permission_is_confirmed_without_user_interaction() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-yolo-permission-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("context.json");
    fs::write(&context_file, "").expect("context file should be created");
    let context_file = context_file.display().to_string();
    let request = crate::provider::LaunchProviderRequest::new(
        "session-yolo",
        "claude",
        "claude-headless",
        "default",
        "claude-opus",
    )
    .with_agent_id("agent-yolo")
    .with_permission_level(crate::provider::AgentPermissionLevel::Yolo);
    let mut run = RuntimeProviderRun::new(
        "provider-run-yolo",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-yolo-permission".to_string(),
            pty_target: Some("test-claude-yolo-permission".to_string()),
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec!["-lc".to_string(), "cat".to_string()],
            pty_env: std::collections::BTreeMap::from([(
                "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                context_file.clone(),
            )]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.pty
        .spawn_for_run(&run)
        .expect("test provider PTY should start");
    let bridge = RecordingPermissionBridge::default();
    let bridge_ref: std::sync::Arc<dyn ProviderNativeInteractionBridge> =
        std::sync::Arc::new(bridge.clone());

    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_terminal_output(
            "session-yolo",
            run.id(),
            &run,
            Some(bridge_ref),
            "Bash command\necho test\nDo you want to proceed?\n1. Yes\n3. No",
        )
        .expect("yolo permission should be confirmed");
    let confirmation_marker = root.join("yolo-rendered-permission-confirmed");
    let first_confirmation = fs::read_to_string(&confirmation_marker)
        .expect("yolo confirmation should leave a suppression marker");
    std::thread::sleep(std::time::Duration::from_millis(5));
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_terminal_output(
            "session-yolo",
            run.id(),
            &run,
            Some(std::sync::Arc::new(bridge.clone())),
            "Bash command\necho test\nDo you want to proceed?\n1. Yes\n3. No",
        )
        .expect("lingering yolo permission should be suppressed");
    assert_eq!(
        fs::read_to_string(&confirmation_marker)
            .expect("yolo confirmation marker should remain present"),
        first_confirmation,
        "the same rendered prompt must not be confirmed twice"
    );
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_terminal_output(
            "session-yolo",
            run.id(),
            &run,
            Some(std::sync::Arc::new(bridge.clone())),
            "Write command\nprintf distinct\nDo you want to proceed?\n1. Yes\n3. No",
        )
        .expect("a distinct yolo permission should be confirmed immediately");
    assert_ne!(
        fs::read_to_string(&confirmation_marker)
            .expect("distinct yolo confirmation marker should be present"),
        first_confirmation,
        "suppression must be scoped to the exact rendered prompt"
    );
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_terminal_output(
            "session-yolo",
            run.id(),
            &run,
            Some(std::sync::Arc::new(bridge.clone())),
            "Claude composer ready",
        )
        .expect("dismissed permission should clear suppression state");
    assert!(
        !confirmation_marker.exists(),
        "a later permission prompt must be eligible for confirmation"
    );

    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        bridge
            .interaction_ids
            .lock()
            .expect("permission interaction recorder should not be poisoned")
            .is_empty(),
        "yolo permission must not create a user interaction"
    );
    assert!(!claude_native_marker(&context_file)
        .as_deref()
        .is_some_and(|marker| marker.starts_with("permission:")));
    app.pty
        .remove_process(run.id())
        .expect("test provider PTY should stop");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn yolo_hook_permission_uses_kernel_policy_when_claude_reports_auto() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-yolo-hook-permission-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    fs::write(&context_file, "").expect("context file should be created");
    let context_file = context_file.display().to_string();
    let request = crate::provider::LaunchProviderRequest::new(
        "session-yolo-hook",
        "claude",
        "claude-headless",
        "default",
        "claude-opus",
    )
    .with_agent_id("agent-yolo-hook")
    .with_permission_level(crate::provider::AgentPermissionLevel::Yolo);
    let run = RuntimeProviderRun::new(
        "provider-run-yolo-hook",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-yolo-hook-permission".to_string(),
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
    let bridge = RecordingPermissionBridge::default();
    let event = serde_json::json!({
        "hook_event_name": "PermissionRequest",
        "hook_context_request_id": "request-auto-yolo",
        "permission_mode": "auto",
        "tool_name": "Bash",
        "tool_input": {
            "command": "rm -rf /tmp/chariox-rev-head && echo \"cleaned\""
        },
    });

    assert!(should_bridge_claude_permission(&event));
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .resolve_permission_event(
            "session-yolo-hook",
            run.id(),
            "agent-yolo-hook",
            &context_file,
            &run,
            Some(std::sync::Arc::new(bridge.clone())),
            &event,
        )
        .expect("yolo hook permission should resolve without user interaction");

    let response: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join("permission-responses/request-auto-yolo.json"))
            .expect("yolo hook should receive an immediate response"),
    )
    .expect("yolo hook response should be valid JSON");
    assert_eq!(response["behavior"], "allow");
    assert_eq!(
        response["message"],
        "Allowed by Chariox yolo permission policy."
    );
    assert!(
        bridge
            .interaction_ids
            .lock()
            .expect("permission interaction recorder should not be poisoned")
            .is_empty(),
        "the provider-reported mode must not override the kernel-owned yolo policy"
    );
    assert!(!claude_native_marker(&context_file)
        .as_deref()
        .is_some_and(|marker| marker.starts_with("permission:")));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_permission_without_an_interaction_bridge_releases_the_waiting_hook() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-permission-passthrough-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    fs::write(&context_file, "").expect("context file should be created");
    let context_file = context_file.display().to_string();
    let request = crate::provider::LaunchProviderRequest::new(
        "session-ask-hook",
        "claude",
        "claude",
        "default",
        "claude-opus",
    )
    .with_agent_id("agent-ask-hook")
    .with_permission_level(crate::provider::AgentPermissionLevel::Required);
    let run = RuntimeProviderRun::new(
        "provider-run-ask-hook",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-ask-hook-permission".to_string(),
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
    let event = serde_json::json!({
        "hook_event_name": "PermissionRequest",
        "hook_context_request_id": "request-no-bridge",
        "permission_mode": "default",
        "tool_name": "Bash",
        "tool_input": { "command": "true" },
    });

    ProviderOutputClaudeNativeBridge::new(&mut app)
        .resolve_permission_event(
            "session-ask-hook",
            run.id(),
            "agent-ask-hook",
            &context_file,
            &run,
            None,
            &event,
        )
        .expect("an unbridged permission should resolve");

    let response: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join("permission-responses/request-no-bridge.json"))
            .expect("the waiting hook should get an immediate response"),
    )
    .expect("the response should be valid JSON");
    assert_eq!(
        response,
        serde_json::json!({}),
        "no decision: Claude asks itself"
    );
    assert!(!claude_native_marker(&context_file)
        .as_deref()
        .is_some_and(|marker| marker.starts_with("permission:")));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn rendered_permission_preserves_reviewable_tool_arguments() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-permission-dedupe-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("context.json");
    fs::write(&context_file, "").expect("context file should be created");
    let context_file = context_file.display().to_string();

    let request = crate::provider::LaunchProviderRequest::new(
        "session-1",
        "claude",
        "claude",
        "default",
        "claude-sonnet",
    )
    .with_agent_id("agent-1")
    .with_permission_level(crate::provider::AgentPermissionLevel::Required);
    let run = RuntimeProviderRun::new(
        "provider-run-1",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-native".to_string(),
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
    let bridge = RecordingPermissionBridge::default();
    let bridge_ref: std::sync::Arc<dyn ProviderNativeInteractionBridge> =
        std::sync::Arc::new(bridge.clone());
    let action = "Tool use integ2-official-fetch - fetch(url: https://developer.mozilla.org/en-US/docs/Web/API/WebSockets_API) (MCP)";
    let mut output = ProviderOutputClaudeNativeBridge::new(&mut app);
    output
        .process_terminal_output(
            "session-1",
            run.id(),
            &run,
            Some(bridge_ref.clone()),
            action,
        )
        .expect("partial tool frame should wait for choices");
    output
        .process_terminal_output(
            "session-1",
            run.id(),
            &run,
            Some(bridge_ref),
            "Do you want to proceed?\n1. Yes\n3. No",
        )
        .expect("rendered permission should be bridged");
    for _ in 0..100 {
        if !bridge.messages.lock().unwrap().is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let messages = bridge.messages.lock().unwrap().clone();
    assert_eq!(messages.len(), 1);
    assert!(
        messages[0].contains(action),
        "MP-08/MP-10/MP-11 permission must show the actual tool and arguments"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn rendered_permission_details_redact_hidden_instructions_and_terminal_controls() {
    let start = crate::provider::NATIVE_TUI_HIDDEN_INSTRUCTIONS_START;
    let end = crate::provider::NATIVE_TUI_HIDDEN_INSTRUCTIONS_END;
    let message = claude_rendered_permission_details(&format!("\x1b[31mTool use fetch(url: https://github.com/charioxai/chariox)\x1b[0m {start}private-first{end} {start}private-second{end} Do you want to proceed?"));
    assert!(message.contains("fetch(url: https://github.com/charioxai/chariox)"));
    assert!(!message.contains("private-") && !message.contains('\x1b'));
    assert!(claude_rendered_permission_details(&"a".repeat(8000)).len() < 4100);
}

#[test]
fn hook_permission_suppresses_post_stop_stale_rendered_permission_fallback() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-permission-dedupe-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("context.json");
    fs::write(&context_file, "").expect("context file should be created");
    let context_file = context_file.display().to_string();

    let request = crate::provider::LaunchProviderRequest::new(
        "session-1",
        "claude",
        "claude",
        "default",
        "claude-sonnet",
    )
    .with_agent_id("agent-1")
    .with_permission_level(crate::provider::AgentPermissionLevel::Required);
    let run = RuntimeProviderRun::new(
        "provider-run-1",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-native".to_string(),
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
    let bridge = RecordingPermissionBridge::default();
    let bridge_ref: std::sync::Arc<dyn ProviderNativeInteractionBridge> =
        std::sync::Arc::new(bridge.clone());
    let event = serde_json::json!({
        "hook_event_name": "PermissionRequest",
        "hook_context_request_id": "request-1",
        "tool_name": "Bash",
        "tool_input": { "command": "echo test" },
    });

    let mut output = ProviderOutputClaudeNativeBridge::new(&mut app);
    output
        .resolve_permission_event(
            "session-1",
            run.id(),
            "agent-1",
            &context_file,
            &run,
            Some(bridge_ref.clone()),
            &event,
        )
        .expect("hook permission should be bridged");
    let attachments = Vec::new();
    output
        .inject_prompt(
            "session-1",
            run.id(),
            "agent-1",
            &context_file,
            &run,
            &ClaudeNativePromptInjection {
                id: "prompt-1",
                origin_prompt_id: "prompt-1",
                prompt: "do the work",
                hidden_system_context: "",
                attachments: &attachments,
                context_handoff: &no_context_handoff,
            },
        )
        .expect("pending permission must not reinject the active prompt");
    std::thread::sleep(std::time::Duration::from_millis(100));
    write_claude_native_marker(&context_file, "");
    output
        .process_terminal_output(
            "session-1",
            run.id(),
            &run,
            Some(bridge_ref),
            "Bash command\necho test\nDo you want to proceed?\n1. Yes\n3. No",
        )
        .expect("post-Stop stale rendered permission should be ignored");

    std::thread::sleep(std::time::Duration::from_millis(100));
    let interaction_ids = bridge
        .interaction_ids
        .lock()
        .expect("permission interaction recorder should not be poisoned")
        .clone();
    assert_eq!(
        interaction_ids,
        vec!["claude-native-permission-provider-run-1-request-1"],
        "one Claude permission must project exactly once across hook and rendered output"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn hook_permission_tombstone_only_consumes_matching_rendered_frame() {
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-permission-tombstone-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("context.json");
    fs::write(&context_file, "").expect("context file should be created");
    let context_file = context_file.display().to_string();
    write_claude_hook_permission_tombstone(
        &context_file,
        &serde_json::json!({
            "tool_name": "Bash",
            "tool_input": { "command": "echo first" },
        }),
    );

    assert!(!take_matching_claude_hook_permission_tombstone(
        &context_file,
        "Bash command\necho second\nDo you want to proceed?\n1. Yes\n3. No",
    ));
    assert!(take_matching_claude_hook_permission_tombstone(
        &context_file,
        "Bash command\necho first\nDo you want to proceed?\n1. Yes\n3. No",
    ));
    assert!(!take_matching_claude_hook_permission_tombstone(
        &context_file,
        "Bash command\necho first\nDo you want to proceed?\n1. Yes\n3. No",
    ));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn rendered_permission_resolution_does_not_reinject_native_prompt() {
    let worktree = crate::test_support::TestWorktree::new("claude-native-permission");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-claude-permission",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-permission-input-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    let events_file = root.join("events.jsonl");
    fs::write(&context_file, "").expect("context file should be created");
    fs::write(&events_file, "").expect("events file should be created");
    let context_file = context_file.display().to_string();

    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude",
        "default",
        "claude-sonnet",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = RuntimeProviderRun::new(
        "provider-run-claude-permission",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-native-permission".to_string(),
            pty_target: Some("test-claude-native-permission".to_string()),
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec!["-lc".to_string(), "cat".to_string()],
            pty_env: std::collections::BTreeMap::from([
                (
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                    context_file.clone(),
                ),
                (
                    "CHARIOX_CLAUDE_NATIVE_EVENTS".to_string(),
                    events_file.display().to_string(),
                ),
            ]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.pty
        .spawn_for_run(&run)
        .expect("test provider PTY should start");
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    let prompt = match app
        .record_native_prompt_started_with_attachments(
            session.id(),
            attachment.id(),
            attachment.id(),
            agent.id(),
            "native permission prompt",
            Vec::new(),
        )
        .expect("native prompt should be recorded")
    {
        crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
        other => panic!("unexpected prompt outcome: {other:?}"),
    };
    write_claude_native_marker(&context_file, "permission:interaction-1");
    write_claude_permission_input(&context_file, "interaction-1", b"1\r");

    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process(session.id(), run.id(), &run, None)
        .expect("permission input should be processed");

    let marker = claude_native_marker(&context_file);
    app.pty
        .remove_process(run.id())
        .expect("test provider PTY should stop");
    let _ = fs::remove_dir_all(root);
    assert_eq!(
        marker.as_deref(),
        Some(format!("permission-resolved:{}", prompt.id()).as_str()),
        "permission approval must keep the active prompt marked as already injected",
    );
}

#[test]
fn headless_stop_stays_active_until_deferred_transcript_drain_finishes() {
    assert_claude_stop_stays_active_until_deferred_transcript_drain_finishes(
        "claude-headless",
        false,
        false,
    );
}

#[test]
fn native_stop_stays_active_until_deferred_semantic_transcript_drain_finishes() {
    assert_claude_stop_stays_active_until_deferred_transcript_drain_finishes(
        "claude", false, false,
    );
}

#[test]
fn late_claude_transcript_drain_does_not_complete_the_next_prompt() {
    assert_claude_stop_stays_active_until_deferred_transcript_drain_finishes(
        "claude-headless",
        true,
        false,
    );
}

#[test]
fn managed_headless_stop_projects_transcript_from_sandbox_account_path() {
    assert_claude_stop_stays_active_until_deferred_transcript_drain_finishes(
        "claude-headless",
        false,
        true,
    );
}

fn assert_claude_stop_stays_active_until_deferred_transcript_drain_finishes(
    provider: &str,
    advance_next_prompt_before_late_drain: bool,
    managed_account_binding: bool,
) {
    use std::io::Write as _;

    let worktree = crate::test_support::TestWorktree::new("claude-native-headless-stop");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-claude-headless-stop",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-headless-stop-test-{}-{}-{}-{}-{}",
        provider,
        advance_next_prompt_before_late_drain,
        managed_account_binding,
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    let events_file = root.join("events.jsonl");
    let transcript_file = root.join("session.jsonl");
    fs::write(&context_file, "").expect("context file should be created");
    let transcript_record = serde_json::json!({
        "type": "assistant",
        "uuid": "assistant-late",
        "message": {
            "id": "message-late",
            "role": "assistant",
            "content": [{ "type": "text", "text": "late final response" }]
        }
    })
    .to_string();
    let transcript_split_at = transcript_record.len() / 2;
    fs::write(&transcript_file, &transcript_record[..transcript_split_at])
        .expect("partial transcript output should be written");
    fs::write(
        &events_file,
        serde_json::json!({
            "hook_event_name": "Stop",
            "transcript_path": if managed_account_binding {
                "/home/chariox/.provider-account/root-1/session.jsonl".to_string()
            } else {
                transcript_file.display().to_string()
            },
        })
        .to_string(),
    )
    .expect("Stop event should be written");
    let context_file = context_file.display().to_string();

    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        provider,
        "default",
        "claude-sonnet",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = RuntimeProviderRun::new(
        "provider-run-claude-headless-stop",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-headless-stop".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: if managed_account_binding {
                {
                    vec![
                        "--bind".to_string(),
                        root.canonicalize()
                            .expect("account fixture should resolve")
                            .display()
                            .to_string(),
                        "/home/chariox/.provider-account/root-1".to_string(),
                        "--setenv".to_string(),
                        crate::provider::MANAGED_PROVIDER_ISOLATION_MARKER_ENV.to_string(),
                        "1".to_string(),
                    ]
                }
            } else {
                Default::default()
            },
            pty_env: std::collections::BTreeMap::from([
                (
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                    context_file.clone(),
                ),
                (
                    "CHARIOX_CLAUDE_NATIVE_EVENTS".to_string(),
                    events_file.display().to_string(),
                ),
            ]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    let prompt = match app
        .record_native_prompt_started_with_attachments(
            session.id(),
            attachment.id(),
            attachment.id(),
            agent.id(),
            "headless prompt",
            Vec::new(),
        )
        .expect("native prompt should be recorded")
    {
        crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
        other => panic!("unexpected prompt outcome: {other:?}"),
    };
    write_claude_native_marker(&context_file, &format!("injected:{}", prompt.id()));
    let queued_prompt = if advance_next_prompt_before_late_drain {
        match app
            .record_native_prompt_started_with_attachments(
                session.id(),
                attachment.id(),
                attachment.id(),
                agent.id(),
                "next headless prompt",
                Vec::new(),
            )
            .expect("next native prompt should be queued")
        {
            crate::session::PromptSubmissionOutcome::Queued { prompt } => Some(prompt),
            other => panic!("unexpected next prompt outcome: {other:?}"),
        }
    } else {
        None
    };

    let outcome = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process(session.id(), run.id(), &run, None)
        .expect("Stop event should be processed");

    assert!(outcome.needs_deferred_transcript_drain);
    assert!(app
        .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
        .expect("active prompt should load")
        .is_some());
    assert!(claude_native_marker(&context_file)
        .as_deref()
        .is_some_and(|marker| marker.starts_with(CLAUDE_TRANSCRIPT_STOP_DRAIN_MARKER_PREFIX)));

    let next_active_prompt_id = if let Some(queued_prompt) = queued_prompt {
        app.prompt_owner_complete_active_prompt_only(session.id(), agent.id())
            .expect("stopped prompt should complete");
        let next = app
            .prompt_owner_activate_next_queued_prompt_for_dispatch(
                session.id(),
                agent.id(),
                Some(queued_prompt.id()),
            )
            .expect("next prompt activation should succeed")
            .expect("next prompt should activate");
        crate::transport::flow_control::note_prompt_started(&mut app, run.id());
        Some(next.id().to_string())
    } else {
        None
    };

    std::fs::OpenOptions::new()
        .append(true)
        .open(&transcript_file)
        .expect("partial transcript should reopen")
        .write_all(&transcript_record.as_bytes()[transcript_split_at..])
        .expect("late transcript output should finish writing");

    ProviderOutputClaudeNativeBridge::new(&mut app)
        .drain_known_claude_transcripts(session.id(), run.id(), &context_file)
        .expect("late transcript output should drain");
    assert!(
        !crate::transport::flow_control::prompt_completion_recorded(&app, run.id()),
        "Claude transcript completion must not compete with the prompt-bound Stop settlement",
    );

    ProviderOutputClaudeNativeBridge::new(&mut app)
        .finish_deferred_stop(session.id(), run.id(), &run)
        .expect("deferred transcript drain should finish");

    let active_prompt = app
        .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
        .expect("settled prompt state should load");
    if let Some(next_active_prompt_id) = next_active_prompt_id {
        assert_eq!(
            active_prompt.as_ref().map(|prompt| prompt.id()),
            Some(next_active_prompt_id.as_str()),
            "the stale Stop and transcript flush must not settle the next prompt",
        );
    } else {
        assert!(active_prompt.is_none());
    }
    let output = app
        .terminal()
        .output_records()
        .into_iter()
        .flat_map(|record| record.bytes)
        .collect::<Vec<_>>();
    assert!(String::from_utf8_lossy(&output).contains("late final response"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_prompt_history_uses_latest_terminal_input_attachment() {
    let app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    app.terminal().record_input(
        "session-1",
        "provider-run-1",
        "attachment-native-a",
        b"first",
    );
    app.terminal()
        .record_input("session-1", "provider-run-2", "attachment-other", b"other");
    app.terminal().record_input(
        "session-1",
        "provider-run-1",
        "attachment-native-b",
        b"second",
    );

    assert_eq!(
        claude_native_history_source_attachment_id(
            &app,
            "session-1",
            "provider-run-1",
            "attachment-fallback",
        ),
        "attachment-native-b"
    );
    assert_eq!(
        claude_native_history_source_attachment_id(
            &app,
            "session-1",
            "provider-run-missing",
            "attachment-fallback",
        ),
        "attachment-fallback"
    );
}

#[test]
fn native_interrupt_control_prompt_is_not_user_history() {
    assert!(claude_native_prompt_is_internal_control(
        "[Request interrupted by user]"
    ));
    assert!(claude_native_prompt_is_internal_control(
        "  [Request interrupted by user]\n"
    ));
    assert!(!claude_native_prompt_is_internal_control(
        "Please explain why the request was interrupted by the user."
    ));
    assert!(claude_native_prompt_is_internal_control(
        "<task-notification><task-id>build-1</task-id><status>completed</status></task-notification>"
    ));
    assert!(claude_native_prompt_is_internal_control(
        "<task-notification><task-id>build-1</task-id></task-notification>\n\
         <task-notification><task-id>test-1</task-id></task-notification>"
    ));
    assert!(!claude_native_prompt_is_internal_control(
        "Review this <task-notification> example for correctness."
    ));
}

#[test]
fn submit_wait_state_defers_enter_until_delay_elapses() {
    let marker = format!("submit-wait:prompt-7:{}", 1_000);
    // Before the settle delay: keep waiting (Enter stays off the lock).
    assert_eq!(
        submit_wait_state(
            Some(&marker),
            "prompt-7",
            1_000 + CLAUDE_SUBMIT_DELAY_MS - 1
        ),
        SubmitWaitState::Waiting
    );
    // At/after the delay: submit the Enter keystroke.
    assert_eq!(
        submit_wait_state(Some(&marker), "prompt-7", 1_000 + CLAUDE_SUBMIT_DELAY_MS),
        SubmitWaitState::ReadyToSubmit
    );
    // A marker for a different prompt is not a submit-wait for this one.
    assert_eq!(
        submit_wait_state(Some(&marker), "prompt-8", 10_000),
        SubmitWaitState::NotSubmitWait
    );
    // Unrelated and malformed markers are ignored.
    assert_eq!(
        submit_wait_state(Some("injected:prompt-7"), "prompt-7", 10_000),
        SubmitWaitState::NotSubmitWait
    );
    // An unparseable timestamp submits rather than stalling forever.
    assert_eq!(
        submit_wait_state(Some("submit-wait:prompt-7:bogus"), "prompt-7", 10_000),
        SubmitWaitState::ReadyToSubmit
    );
}

#[test]
fn claude_headless_dispatch_waits_for_user_prompt_submit_acknowledgement() {
    assert_claude_dispatch_waits_for_user_prompt_submit("claude-headless");
}

#[test]
fn claude_native_dispatch_waits_for_user_prompt_submit_acknowledgement() {
    assert_claude_dispatch_waits_for_user_prompt_submit("claude");
}

fn assert_claude_dispatch_waits_for_user_prompt_submit(provider: &str) {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let root = std::env::temp_dir().join(format!(
        "chariox-{provider}-ack-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    let events_file = root.join("events.jsonl");
    fs::write(&context_file, "").expect("context file should be created");
    fs::write(&events_file, "").expect("events file should be created");
    let context_file = context_file.display().to_string();
    let request = crate::provider::LaunchProviderRequest::new(
        "session-1",
        "claude",
        provider,
        "default",
        "claude-sonnet",
    )
    .with_agent_id("agent-1")
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let run = RuntimeProviderRun::new(
        "provider-run-1",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-headless-ack".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::from([
                (
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                    context_file.clone(),
                ),
                (
                    "CHARIOX_CLAUDE_NATIVE_EVENTS".to_string(),
                    events_file.display().to_string(),
                ),
            ]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    let prompt_id = "prompt-ack";
    write_claude_native_marker(&context_file, &format!("injected:{prompt_id}"));
    write_claude_headless_submit_retry(
        &context_file,
        prompt_id,
        0,
        unix_epoch_ms(),
        "Explain the lifecycle briefly.",
    );
    let dispatch = KernelPromptDispatch {
        session_id: "session-1".to_string(),
        provider_run_id: run.id().to_string(),
        agent_id: "agent-1".to_string(),
        prompt_id: prompt_id.to_string(),
        target_active_prompt_id: None,
        source_attachment_id: "attachment-1".to_string(),
        prompt: "Explain the lifecycle briefly.".to_string(),
        hidden_system_context: String::new(),
        attachments: Vec::new(),
        prompt_origin: crate::session::PromptOrigin::Chariox,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        steering: false,
    };

    let injected = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_prompt_dispatch_attempt(
            "session-1",
            run.id(),
            &run,
            &dispatch,
            &no_context_handoff,
        )
        .expect("injected prompt should remain pending");
    assert_eq!(injected, ClaudeNativeDispatchAttempt::AwaitingInjection);

    fs::write(
        &events_file,
        serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "prompt": "An unrelated native user draft.",
        })
        .to_string(),
    )
    .unwrap();
    assert_eq!(
        ProviderOutputClaudeNativeBridge::new(&mut app)
            .process_prompt_dispatch_attempt(
                "session-1",
                run.id(),
                &run,
                &dispatch,
                &no_context_handoff
            )
            .unwrap(),
        ClaudeNativeDispatchAttempt::AwaitingInjection,
        "an unrelated hook must not acknowledge the managed prompt",
    );

    fs::write(
        &events_file,
        [
            serde_json::json!({
                "hook_event_name": "Stop",
            })
            .to_string(),
            serde_json::json!({
                "hook_event_name": "UserPromptSubmit",
                "prompt": "Explain the lifecycle briefly.",
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("hook events should be written");
    let accepted = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_prompt_dispatch_attempt(
            "session-1",
            run.id(),
            &run,
            &dispatch,
            &no_context_handoff,
        )
        .expect("hook-acknowledged prompt should complete dispatch");
    assert_eq!(accepted, ClaudeNativeDispatchAttempt::Completed);
    assert!(
        fs::read_to_string(&events_file)
            .expect("events should remain available to the normal output bridge")
            .contains("Stop"),
        "dispatch acknowledgement must not consume later lifecycle events"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn claude_headless_steering_dispatch_waits_for_provider_acknowledgement() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-headless-steering-injection-test-{}-{}",
        std::process::id(),
        unix_epoch_ms()
    ));
    fs::create_dir_all(&root).expect("temporary directory should exist");
    let context_file = root.join("hidden-context.txt");
    fs::write(&context_file, "").expect("context file should exist");
    let request = crate::provider::LaunchProviderRequest::new(
        "session-1",
        "claude",
        "claude-headless",
        "default",
        "claude-opus-4-7",
    )
    .with_agent_id("agent-1");
    let run = crate::provider::RuntimeProviderRun::new(
        "provider-run-1",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-headless-steering-injection".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::from([(
                "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                context_file.to_string_lossy().into_owned(),
            )]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    let dispatch = KernelPromptDispatch {
        session_id: "session-1".to_string(),
        provider_run_id: run.id().to_string(),
        agent_id: "agent-1".to_string(),
        prompt_id: "pending-steering-1".to_string(),
        target_active_prompt_id: Some("prompt-1".to_string()),
        source_attachment_id: "attachment-1".to_string(),
        prompt: "Add one sentence before finishing.".to_string(),
        hidden_system_context: String::new(),
        attachments: Vec::new(),
        prompt_origin: crate::session::PromptOrigin::Chariox,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        steering: true,
    };

    write_claude_native_marker(
        &context_file.to_string_lossy(),
        "injected:pending-steering-1",
    );
    let injected = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_prompt_dispatch_attempt(
            "session-1",
            run.id(),
            &run,
            &dispatch,
            &no_context_handoff,
        )
        .expect("injected steering should remain pending");

    assert_eq!(injected, ClaudeNativeDispatchAttempt::AwaitingInjection);

    write_claude_native_marker(
        &context_file.to_string_lossy(),
        "accepted:pending-steering-1",
    );
    let accepted = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_prompt_dispatch_attempt(
            "session-1",
            run.id(),
            &run,
            &dispatch,
            &no_context_handoff,
        )
        .expect("acknowledged steering should complete dispatch");

    assert_eq!(accepted, ClaudeNativeDispatchAttempt::Completed);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn claude_headless_user_prompt_submit_acknowledges_matching_managed_dispatches() {
    let worktree = crate::test_support::TestWorktree::new("claude-native-submit-ack");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-submit-wait-ack",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("session should attach");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-headless-submit-wait-ack-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    let events_file = root.join("events.jsonl");
    fs::write(&context_file, "").expect("context file should be created");
    fs::write(
        &events_file,
        serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "prompt": "A stale prompt from the active turn.",
        })
        .to_string(),
    )
    .expect("UserPromptSubmit event should be written");
    let context_file = context_file.display().to_string();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude-headless",
        "default",
        "claude-sonnet",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = RuntimeProviderRun::new(
        "provider-run-submit-wait-ack",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-headless-submit-wait-ack".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::from([
                (
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                    context_file.clone(),
                ),
                (
                    "CHARIOX_CLAUDE_NATIVE_EVENTS".to_string(),
                    events_file.display().to_string(),
                ),
            ]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    let active_prompt = match app
        .record_native_prompt_started_with_attachments(
            session.id(),
            attachment.id(),
            attachment.id(),
            agent.id(),
            "Run the web tests and summarize them.",
            Vec::new(),
        )
        .expect("active prompt should start")
    {
        crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
        other => panic!("unexpected prompt outcome: {other:?}"),
    };
    let steering_prompt_id = "leased-steer:steer-1";
    let steering_marker = format!("submit-wait:{steering_prompt_id}:{}", unix_epoch_ms());
    write_claude_native_marker(&context_file, &steering_marker);
    write_claude_headless_submit_retry(
        &context_file,
        steering_prompt_id,
        0,
        unix_epoch_ms(),
        "Also report the package version.",
    );

    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process(session.id(), run.id(), &run, None)
        .expect("stale UserPromptSubmit should be consumed");
    assert_eq!(
        claude_native_marker(&context_file).as_deref(),
        Some(steering_marker.as_str())
    );

    fs::write(
        &events_file,
        serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "prompt": "Also report the package version.",
        })
        .to_string(),
    )
    .expect("matching UserPromptSubmit event should be written");
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process(session.id(), run.id(), &run, None)
        .expect("active UserPromptSubmit should be acknowledged");

    assert_eq!(
        claude_native_marker(&context_file).as_deref(),
        Some(format!("accepted:{steering_prompt_id}").as_str())
    );
    let projected = app
        .sessions()
        .get_session(session.id())
        .expect("session should remain available");
    assert_eq!(
        projected
            .active_prompt_for_agent(agent.id())
            .map(|prompt| prompt.id()),
        Some(active_prompt.id())
    );
    assert!(projected
        .queued_prompts_for_agent(agent.id())
        .is_none_or(|queue| queue.is_empty()));

    app.prompt_owner_cancel_active_prompt_only(session.id(), agent.id())
        .expect("managed prompt should cancel before its late hook acknowledgement");
    write_claude_native_marker(&context_file, &format!("injected:{}", active_prompt.id()));
    write_claude_headless_submit_retry(
        &context_file,
        active_prompt.id(),
        0,
        unix_epoch_ms(),
        "Run the web tests and summarize them.",
    );
    let terminal_input_count = app.terminal().input_records().len();
    fs::write(
        &events_file,
        serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "prompt": "Run the web tests and summarize them.",
        })
        .to_string(),
    )
    .expect("late matching UserPromptSubmit event should be written");
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process(session.id(), run.id(), &run, None)
        .expect("late managed UserPromptSubmit should be consumed");

    assert_eq!(
        claude_native_marker(&context_file).as_deref(),
        Some(format!("accepted:{}", active_prompt.id()).as_str())
    );
    assert_eq!(app.terminal().input_records().len(), terminal_input_count);
    assert!(app
        .sessions()
        .get_session(session.id())
        .expect("session should remain available")
        .active_prompt_for_agent(agent.id())
        .is_none());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn claude_workspace_trust_waits_for_approval_before_exactly_once_dispatch() {
    workspace_trust_approval_fixture("1. No, exit\n2. Yes, I trust this folder", "\x1b[B\n");
}

#[test]
fn claude_workspace_trust_accepts_yes_first_without_selecting_no() {
    workspace_trust_approval_fixture("❯ 1. Yes, I trust this folder\n2. No, exit", "\n");
}

#[test]
fn claude_workspace_trust_keeps_highlighted_yes_second() {
    workspace_trust_approval_fixture("1. No, exit\n❯ 2. Yes, I trust this folder", "\n");
}

// MP-08/MP-10/MP-11: captured official Claude 2.1.292 uses unnumbered choices.
#[test]
fn claude_workspace_trust_accepts_unnumbered_no_first() {
    workspace_trust_approval_fixture("❯ No, exit\nYes, I trust this folder", "\x1b[B\n");
}

#[test]
fn claude_workspace_trust_accepts_unnumbered_yes_first() {
    workspace_trust_approval_fixture("❯ Yes, I trust this folder\nNo, exit", "\n");
}

#[test]
fn claude_workspace_trust_keeps_unnumbered_highlighted_yes() {
    workspace_trust_approval_fixture("No, exit\n❯ Yes, I trust this folder", "\n");
}

#[test]
fn claude_workspace_trust_selects_yes_above_highlighted_no() {
    workspace_trust_approval_fixture("1. Yes, I trust this folder\n❯ 2. No, exit", "\x1b[A\n");
}

#[test]
fn claude_workspace_trust_approval_reads_the_completed_frame() {
    workspace_trust_approval_frame("Yes, I trust this folder\nNo, exit", "\x1b[A\n", 1);
}

#[test]
fn claude_workspace_trust_unparseable_approval_is_not_denial() {
    workspace_trust_approval_frame("Yes, I trust this folder\nNo, exit", "", 2);
}

fn workspace_trust_approval_fixture(choices: &str, expected_input: &str) {
    workspace_trust_approval_frame(choices, expected_input, 0);
}

fn workspace_trust_approval_frame(choices: &str, expected_input: &str, frame_mode: u8) {
    let worktree = crate::test_support::TestWorktree::new("claude-native-trust-approval");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-startup-trust-approval",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-startup-trust-approval-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    let events_file = root.join("events.jsonl");
    let capture_file = root.join("pty-input.log");
    fs::write(&context_file, "").expect("context file should be created");
    fs::write(&events_file, "").expect("events file should be created");
    let run = startup_readiness_run(
        session.id(),
        agent.id(),
        "provider-run-startup-trust-approval",
        &context_file,
        &events_file,
        format!("tee {} >/dev/null", capture_file.display()),
    );
    let context_file = context_file.display().to_string();
    let mut run = run;
    run.mark_running();
    app.pty
        .spawn_for_run(&run)
        .expect("test provider PTY should start");
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    let prompt = match app
        .record_native_prompt_started_with_attachments(
            session.id(),
            attachment.id(),
            attachment.id(),
            agent.id(),
            "startup-gated-task-unique",
            Vec::new(),
        )
        .expect("queued task should become active")
    {
        crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
        other => panic!("unexpected prompt outcome: {other:?}"),
    };
    let dispatch = KernelPromptDispatch {
        session_id: session.id().to_string(),
        provider_run_id: run.id().to_string(),
        agent_id: agent.id().to_string(),
        prompt_id: prompt.id().to_string(),
        target_active_prompt_id: None,
        source_attachment_id: attachment.id().to_string(),
        prompt: prompt.prompt().to_string(),
        hidden_system_context: String::new(),
        attachments: Vec::new(),
        prompt_origin: crate::session::PromptOrigin::Chariox,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        steering: false,
    };
    let bridge = StartupTrustBridge::default();
    let trust_frame = format!("Quick safety check\nDo you trust this folder?\n{choices}");
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_terminal_output(
            session.id(),
            run.id(),
            &run,
            Some(std::sync::Arc::new(bridge.clone())),
            &trust_frame,
        )
        .expect("startup trust should enter the native interaction path");

    let interaction = bridge.wait_for_interaction();
    assert_eq!(interaction.kind(), RuntimeInteractionKind::Permission);
    assert_eq!(interaction.default_on_timeout(), Some("deny"));
    assert!(interaction
        .message()
        .contains("queued task will not be sent"));
    assert_eq!(interaction.choices().len(), 2);
    let marker = claude_native_marker(&context_file).expect("trust marker should be present");
    assert!(marker.starts_with("startup-workspace-trust:"), "{marker}");
    assert!(
        !marker.starts_with("startup-wait:"),
        "trust must not default to No"
    );
    assert!(fs::read_to_string(&context_file)
        .expect("context file should remain readable")
        .is_empty());

    let pending = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_prompt_dispatch_attempt(
            session.id(),
            run.id(),
            &run,
            &dispatch,
            &no_context_handoff,
        )
        .expect("unapproved startup trust should remain pending");
    assert_eq!(pending, ClaudeNativeDispatchAttempt::AwaitingInjection);
    assert!(claude_native_marker(&context_file)
        .as_deref()
        .is_some_and(|value| value.starts_with("startup-workspace-trust:")));
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        fs::read_to_string(&capture_file)
            .unwrap_or_default()
            .is_empty(),
        "no PTY bytes may be sent before trust approval"
    );

    if frame_mode == 1 {
        ProviderOutputClaudeNativeBridge::new(&mut app)
            .process_terminal_output(
                session.id(),
                run.id(),
                &run,
                Some(std::sync::Arc::new(bridge.clone())),
                "\n❯ No, exit",
            )
            .unwrap();
    }
    if frame_mode == 2 {
        fs::write(
            claude_permission_recent_file(&context_file).unwrap(),
            "Quick safety check: choices unavailable",
        )
        .unwrap();
    }
    // MP-08 / MP-10: a human can answer after the provider has gone quiet.
    // The mailbox write must wake the shared owner without another PTY byte.
    let output_ready = app.providers.output_ready_signal();
    output_ready.take_ready_provider_run_ids();
    let before_reply = output_ready.sequence();
    bridge.resolve("allow_once", "allow");
    let wake_deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    while output_ready.sequence() == before_reply && std::time::Instant::now() < wake_deadline {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(
        output_ready
            .take_ready_provider_run_ids()
            .contains(run.id()),
        "MP-08 / MP-10: a resolved workspace trust mailbox must wake this run's shared pump"
    );

    if frame_mode == 2 {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let result = ProviderOutputClaudeNativeBridge::new(&mut app)
                .process_prompt_dispatch_attempt(
                    session.id(),
                    run.id(),
                    &run,
                    &dispatch,
                    &no_context_handoff,
                );
            if let Err(error) = result {
                let message = error.to_string();
                assert!(
                    message.contains("could not select"),
                    "MP-08/MP-11: explicit approval is not denial: {message}"
                );
                assert!(!message.contains("trust was denied"));
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.pty.remove_process(run.id()).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        return;
    }
    let mut approved = false;
    for _ in 0..500 {
        let _ = ProviderOutputClaudeNativeBridge::new(&mut app)
            .process_prompt_dispatch_attempt(
                session.id(),
                run.id(),
                &run,
                &dispatch,
                &no_context_handoff,
            )
            .expect("approval should be consumed through dispatch");
        if claude_native_marker(&context_file)
            .as_deref()
            .is_some_and(|value| value.starts_with("startup-wait:"))
        {
            approved = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(approved, "explicit approval should reach startup wait");
    for _ in 0..100 {
        if fs::read_to_string(&capture_file)
            .unwrap_or_default()
            .contains(expected_input)
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        fs::read_to_string(&capture_file)
            .expect("PTY should capture workspace approval")
            .contains(expected_input),
        "approval must select Yes for the rendered choices"
    );
    if expected_input == "\n" {
        assert!(
            !fs::read_to_string(&capture_file)
                .unwrap()
                .contains("\x1b[B"),
            "an already selected Yes must not move to No"
        );
    }

    std::thread::sleep(std::time::Duration::from_millis(4_100));
    fs::write(
        root.join("permission-recent.txt"),
        "Claude Code ❯ ⏵⏵ mode (shift+tab to cycle)",
    )
    .expect("composer fixture should be written");
    let output_ready = app.providers.output_ready_signal();
    output_ready.take_ready_provider_run_ids();
    let typed = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_prompt_dispatch_attempt(
            session.id(),
            run.id(),
            &run,
            &dispatch,
            &no_context_handoff,
        )
        .expect("approved task should be typed through dispatch");
    assert_eq!(typed, ClaudeNativeDispatchAttempt::AwaitingInjection);
    assert!(claude_native_marker(&context_file)
        .as_deref()
        .is_some_and(|value| value.starts_with("submit-wait:")));
    assert!(output_ready.next_due_at_ms().is_some(),
        "MP-08 / MP-10: the first visible prompt must schedule its deferred Enter without PTY output");

    std::thread::sleep(std::time::Duration::from_millis(300));
    let submitted = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_prompt_dispatch_attempt(
            session.id(),
            run.id(),
            &run,
            &dispatch,
            &no_context_handoff,
        )
        .expect("approved task should submit exactly once");
    assert_eq!(submitted, ClaudeNativeDispatchAttempt::AwaitingInjection);
    assert_eq!(
        claude_native_marker(&context_file).as_deref(),
        Some(format!("injected:{}", prompt.id()).as_str())
    );

    let retry = read_claude_headless_submit_retry(&context_file);
    assert_eq!(retry.prompt_id, prompt.id());
    fs::write(
        &events_file,
        serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "prompt": retry.visible_prompt,
        })
        .to_string(),
    )
    .expect("provider acknowledgement should be written");
    let completed = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_prompt_dispatch_attempt(
            session.id(),
            run.id(),
            &run,
            &dispatch,
            &no_context_handoff,
        )
        .expect("provider acknowledgement should complete dispatch");
    assert_eq!(completed, ClaudeNativeDispatchAttempt::Completed);
    assert_eq!(
        claude_native_marker(&context_file).as_deref(),
        Some(format!("accepted:{}", prompt.id()).as_str())
    );
    let repeated = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_prompt_dispatch_attempt(
            session.id(),
            run.id(),
            &run,
            &dispatch,
            &no_context_handoff,
        )
        .expect("completed dispatch should remain idempotent");
    assert_eq!(repeated, ClaudeNativeDispatchAttempt::Completed);

    for _ in 0..100 {
        if let Ok(captured) = fs::read_to_string(&capture_file) {
            if captured.matches("startup-gated-task-unique").count() == 1 {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let captured = fs::read_to_string(&capture_file).expect("PTY should capture dispatched task");
    assert_eq!(
        captured.matches("startup-gated-task-unique").count(),
        1,
        "the real PTY dispatch must contain the task exactly once: {captured:?}"
    );
    app.pty
        .remove_process(run.id())
        .expect("test provider PTY should stop");
    let _ = fs::remove_dir_all(root);
}

fn no_context_handoff(_room: usize) -> String {
    String::new()
}

#[test]
fn claude_native_handoff_takes_only_the_room_its_turn_leaves() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-native-handoff-room-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    let events_file = root.join("events.jsonl");
    let capture_file = root.join("pty-input.log");
    fs::write(&context_file, "").expect("context file should be created");
    fs::write(&events_file, "").expect("events file should be created");
    let request = crate::provider::LaunchProviderRequest::new(
        "session-1",
        "claude",
        "claude",
        "default",
        "claude-opus",
    )
    .with_agent_id("agent-1")
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = RuntimeProviderRun::new(
        "provider-run-handoff-room",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "provider-run-handoff-room-process".to_string(),
            pty_target: Some("provider-run-handoff-room".to_string()),
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec![
                "-lc".to_string(),
                format!("tee {} >/dev/null", capture_file.display()),
            ],
            pty_env: std::collections::BTreeMap::from([
                (
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                    context_file.display().to_string(),
                ),
                (
                    "CHARIOX_CLAUDE_NATIVE_EVENTS".to_string(),
                    events_file.display().to_string(),
                ),
            ]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.pty
        .spawn_for_run(&run)
        .expect("test provider PTY should start");
    app.providers_mut().insert_run_for_test(run.clone());
    let context_file = context_file.display().to_string();
    let workflow_context = "w".repeat(30_000);
    let rooms = std::cell::RefCell::new(Vec::new());
    // The worst case: a handoff that fills the room with two-byte scalars.
    let handoff = |room: usize| {
        rooms.borrow_mut().push(room);
        "é".repeat(room / 2)
    };
    let attachments = Vec::new();

    ProviderOutputClaudeNativeBridge::new(&mut app)
        .inject_prompt(
            "session-1",
            run.id(),
            "agent-1",
            &context_file,
            &run,
            &ClaudeNativePromptInjection {
                id: "prompt-1",
                origin_prompt_id: "prompt-1",
                prompt: "continue",
                hidden_system_context: &workflow_context,
                attachments: &attachments,
                context_handoff: &handoff,
            },
        )
        .expect("a handoff within the remaining room should dispatch");

    let room = rooms.borrow()[0];
    assert!(room > 10_000 && room < 18_000, "{room}");
    let hidden = fs::read_to_string(&context_file).expect("hidden context should be written");
    assert!(hidden.starts_with(&workflow_context));
    assert!(hidden.ends_with(&"é".repeat(room / 2)));
    crate::provider::ensure_claude_native_hidden_context_fits(run.id(), &hidden)
        .expect("the written context should fit the hook delivery");
    app.pty
        .remove_process(run.id())
        .expect("test provider PTY should stop");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn claude_headless_early_exit_before_ack_has_bounded_diagnostic() {
    let worktree = crate::test_support::TestWorktree::new("claude-native-early-exit");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-startup-early-exit-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    let events_file = root.join("events.jsonl");
    fs::write(&context_file, "").expect("context file should be created");
    fs::write(&events_file, "").expect("events file should be created");
    let mut run = startup_readiness_run(
        session.id(),
        agent.id(),
        "provider-run-startup-early-exit",
        &context_file,
        &events_file,
        "exit 1".to_string(),
    );
    run.mark_running();
    app.pty
        .spawn_for_run(&run)
        .expect("test provider PTY should start");
    app.providers_mut().insert_run_for_test(run.clone());
    for _ in 0..100 {
        if matches!(
            app.pty.poll_process_state(run.id()),
            Ok(crate::pty::PtyProcessState::Exited { .. })
        ) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let dispatch = KernelPromptDispatch {
        session_id: session.id().to_string(),
        provider_run_id: run.id().to_string(),
        agent_id: agent.id().to_string(),
        prompt_id: "early-exit-prompt".to_string(),
        target_active_prompt_id: None,
        source_attachment_id: "early-exit-attachment".to_string(),
        prompt: "early-exit-task-should-not-send".to_string(),
        hidden_system_context: String::new(),
        attachments: Vec::new(),
        prompt_origin: crate::session::PromptOrigin::Chariox,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        steering: false,
    };
    let error = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_prompt_dispatch_attempt(
            session.id(),
            run.id(),
            &run,
            &dispatch,
            &no_context_handoff,
        )
        .expect_err("an exited provider must fail before native dispatch");
    assert!(error
        .to_string()
        .contains("Provider exited before the prompt was delivered:"));
    assert!(
        error.to_string().contains("status 1"),
        "MP-08 / MP-10: an early exit must preserve the real child status: {error}"
    );
    assert!(
        error.to_string().contains("Retry"),
        "a failed first turn needs recovery"
    );
    assert!(fs::read_to_string(&context_file)
        .expect("context file should remain readable")
        .is_empty());
    let _ = app.pty.remove_process(run.id());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn claude_headless_dispatch_observes_ask_user_question_queue_acknowledgement() {
    let worktree = crate::test_support::TestWorktree::new("claude-native-ask-user");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-ask-user-question-ack",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("session should attach");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-headless-ask-user-question-ack-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    let events_file = root.join("events.jsonl");
    let transcript_file = root.join("transcript.jsonl");
    fs::write(&context_file, "").expect("context file should be created");
    fs::write(
        &transcript_file,
        [
            serde_json::json!({
                "type": "assistant",
                "uuid": "ask-user-question",
                "message": {
                    "id": "msg-ask-user-question",
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "id": "toolu-ask-user-question",
                        "name": "AskUserQuestion",
                        "input": { "question": "Which topology should the aggregate use?" }
                    }]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "user",
                "uuid": "ask-user-question-response",
                "message": {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "toolu-ask-user-question",
                        "content": "The user answered the questions:"
                    }]
                },
                "toolUseResult": {
                    "answers": {
                        "Which topology should the aggregate use?":
                            "Use HOME=/home/chariox and continue the aggregate."
                    }
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "queue-operation",
                "operation": "enqueue",
                "content": "Use HOME=/home/chariox and continue the aggregate."
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("transcript fixture should be written");
    fs::write(
        &events_file,
        serde_json::json!({
            "hook_event_name": "SessionStart",
            "transcript_path": transcript_file.display().to_string(),
        })
        .to_string(),
    )
    .expect("transcript hook fixture should be written");
    let context_file = context_file.display().to_string();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude-headless",
        "default",
        "claude-opus",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = RuntimeProviderRun::new(
        "provider-run-ask-user-question-ack",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-claude-headless-ask-user-question-ack".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::from([
                (
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                    context_file.clone(),
                ),
                (
                    "CHARIOX_CLAUDE_NATIVE_EVENTS".to_string(),
                    events_file.display().to_string(),
                ),
            ]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.record_native_prompt_started_with_attachments(
        session.id(),
        attachment.id(),
        attachment.id(),
        agent.id(),
        "Inspect the aggregate and ask for the topology decision.",
        Vec::new(),
    )
    .expect("active prompt should start");

    let steering_prompt_id = "steering-topology-decision";
    let steering_prompt = "Use HOME=/home/chariox and continue the aggregate.";
    write_claude_native_marker(&context_file, &format!("injected:{steering_prompt_id}"));
    write_claude_headless_submit_retry(
        &context_file,
        steering_prompt_id,
        0,
        unix_epoch_ms(),
        steering_prompt,
    );
    let dispatch = KernelPromptDispatch {
        session_id: session.id().to_string(),
        provider_run_id: run.id().to_string(),
        agent_id: agent.id().to_string(),
        prompt_id: steering_prompt_id.to_string(),
        target_active_prompt_id: app
            .sessions()
            .get_session(session.id())
            .expect("session should remain available")
            .active_prompt_for_agent(agent.id())
            .map(|prompt| prompt.id().to_string()),
        source_attachment_id: attachment.id().to_string(),
        prompt: steering_prompt.to_string(),
        hidden_system_context: String::new(),
        attachments: Vec::new(),
        prompt_origin: crate::session::PromptOrigin::Chariox,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        steering: true,
    };

    let outcome = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_prompt_dispatch_attempt(
            session.id(),
            run.id(),
            &run,
            &dispatch,
            &no_context_handoff,
        )
        .expect("AskUserQuestion queue acknowledgement should be processed");

    assert_eq!(outcome, ClaudeNativeDispatchAttempt::Completed);
    assert_eq!(
        claude_native_marker(&context_file).as_deref(),
        Some(format!("accepted:{steering_prompt_id}").as_str()),
        "the dispatch loop must observe Claude's matching queue record without waiting for the output pump"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn claude_native_dispatch_marker_extracts_steering_identity() {
    assert_eq!(
        claude_native_dispatch_prompt_id("injected:steering-prompt-1"),
        Some("steering-prompt-1")
    );
    assert_eq!(
        claude_native_dispatch_prompt_id("submit-wait:steering-prompt-1:1234"),
        Some("steering-prompt-1")
    );
    assert_eq!(claude_native_dispatch_prompt_id("startup-wait:1234"), None);
}

#[test]
fn claude_headless_steering_enqueue_acknowledges_exact_injected_prompt() {
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-headless-steering-enqueue-ack-test-{}-{}",
        std::process::id(),
        unix_epoch_ms()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    fs::write(&context_file, "").expect("context file should be created");
    let context_file = context_file.display().to_string();
    let steering_prompt_id = "leased-steer:steer-1";
    let steering_prompt = "Also report the package version.";
    write_claude_native_marker(&context_file, &format!("injected:{steering_prompt_id}"));
    write_claude_headless_submit_retry(
        &context_file,
        steering_prompt_id,
        0,
        unix_epoch_ms(),
        steering_prompt,
    );

    acknowledge_claude_headless_steering_enqueue(
        &context_file,
        Some("active-prompt-1"),
        &["A different queued prompt.".to_string()],
    );
    assert_eq!(
        claude_native_marker(&context_file).as_deref(),
        Some(format!("injected:{steering_prompt_id}").as_str())
    );

    acknowledge_claude_headless_steering_enqueue(
        &context_file,
        Some("active-prompt-1"),
        &[steering_prompt.to_string()],
    );
    assert_eq!(
        claude_native_marker(&context_file).as_deref(),
        Some(format!("accepted:{steering_prompt_id}").as_str())
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn claude_transcript_drain_maps_assistant_text_reasoning_and_tools() {
    let mut cursor = ClaudeTranscriptCursor::default();
    let dir = std::env::temp_dir().join(format!(
        "chariox-claude-transcript-test-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    let transcript = dir.join("session.jsonl");
    fs::write(
            &transcript,
            [
                serde_json::json!({
                    "type": "assistant",
                    "uuid": "assistant-1",
                    "sessionId": "claude-session-1",
                    "message": {
                        "id": "msg_1",
                        "model": "claude-sonnet-4-6",
                        "role": "assistant",
                        "stop_reason": "tool_use",
                        "content": [
                            { "type": "thinking", "thinking": "considering" },
                            { "type": "text", "text": "hello" },
                            { "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": { "command": "pwd" } }
                        ]
                    }
                })
                .to_string(),
                serde_json::json!({
                    "type": "user",
                    "uuid": "user-1",
                    "message": {
                        "role": "user",
                        "content": [
                            { "type": "tool_result", "tool_use_id": "toolu_1", "content": "ok" }
                        ]
                    }
                })
                .to_string(),
            ]
            .join("\n"),
        )
        .expect("fixture should write");

    let drain = drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);

    assert_eq!(drain.session_id.as_deref(), Some("claude-session-1"));
    assert_eq!(drain.model.as_deref(), Some("claude/claude-sonnet-4-6"));
    assert_eq!(drain.assistant_message_ids, vec!["msg_1"]);
    assert!(drain.terminal_assistant_message_ids.is_empty());
    assert_eq!(drain.chunks.len(), 4);
    assert_eq!(drain.chunks[0].kind, TerminalOutputKind::ProviderReasoning);
    assert_eq!(drain.chunks[0].text, "considering");
    assert_eq!(drain.chunks[1].kind, TerminalOutputKind::ProviderOutput);
    assert_eq!(drain.chunks[1].text, "hello");
    assert_eq!(drain.chunks[2].kind, TerminalOutputKind::ProviderTool);
    assert!(drain.chunks[2].text.contains("\"tool\":\"Bash\""));
    assert!(drain.chunks[3].text.contains("\"status\":\"completed\""));

    let second = drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);
    assert!(second.chunks.is_empty());
    assert!(second.assistant_message_ids.is_empty());

    let mut transcript_text = fs::read_to_string(&transcript).expect("fixture should read");
    transcript_text.push('\n');
    transcript_text.push_str(
        &serde_json::json!({
            "type": "assistant",
            "uuid": "assistant-final",
            "message": {
                "id": "msg_final",
                "model": "claude-sonnet-4-6",
                "role": "assistant",
                "stop_reason": "stop_sequence",
                "content": [{ "type": "text", "text": "done" }]
            }
        })
        .to_string(),
    );
    transcript_text.push('\n');
    fs::write(&transcript, transcript_text).expect("terminal fixture should append");

    let terminal = drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);
    assert_eq!(terminal.assistant_message_ids, vec!["msg_final"]);
    assert_eq!(terminal.terminal_assistant_message_ids, vec!["msg_final"]);

    let after_terminal =
        drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);
    assert!(after_terminal.terminal_assistant_message_ids.is_empty());

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn claude_transcript_drain_classifies_session_limit_without_success_completion() {
    let mut cursor = ClaudeTranscriptCursor::default();
    let dir = std::env::temp_dir().join(format!(
        "chariox-claude-transcript-session-limit-test-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    let transcript = dir.join("session.jsonl");
    fs::write(
        &transcript,
        serde_json::json!({
            "type": "assistant",
            "uuid": "assistant-session-limit",
            "sessionId": "claude-session-limit",
            "message": {
                "id": "message-session-limit",
                "model": "claude-opus-4-8",
                "role": "assistant",
                "stop_reason": "stop_sequence",
                "content": [{
                    "type": "text",
                    "text": "You've hit your session limit · resets 10:40pm (UTC)"
                }]
            }
        })
        .to_string(),
    )
    .expect("fixture should write");

    let drain = drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);

    assert_eq!(drain.chunks.len(), 1);
    assert_eq!(drain.assistant_message_ids, vec!["message-session-limit"]);
    assert_eq!(
        drain.terminal_assistant_message_ids,
        vec!["message-session-limit"]
    );
    assert!(drain
        .terminal_failure
        .as_deref()
        .is_some_and(|failure| failure.contains("substitutable resource limit")));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn claude_transcript_session_limit_projects_error_and_preserves_diagnostic() {
    let worktree = crate::test_support::TestWorktree::new("claude-native-transcript-limit");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-claude-transcript-limit",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-transcript-limit-projection-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    let transcript_file = root.join("session.jsonl");
    fs::write(&context_file, "").expect("context should be created");
    fs::write(
        &transcript_file,
        serde_json::json!({
            "type": "assistant",
            "uuid": "assistant-session-limit",
            "sessionId": "claude-session-limit",
            "message": {
                "id": "message-session-limit",
                "model": "claude-opus-4-8",
                "role": "assistant",
                "stop_reason": "stop_sequence",
                "content": [{
                    "type": "text",
                    "text": "You've hit your session limit · resets 10:40pm (UTC)"
                }]
            }
        })
        .to_string(),
    )
    .expect("transcript should be written");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude-headless",
        "default",
        "claude-opus-4-8",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = RuntimeProviderRun::new(
        "provider-run-claude-transcript-limit",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "claude-transcript-limit".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::from([(
                "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                context_file.display().to_string(),
            )]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");

    let failure = ProviderOutputClaudeNativeBridge::new(&mut app)
        .drain_claude_transcript(
            session.id(),
            run.id(),
            &context_file.display().to_string(),
            &transcript_file.display().to_string(),
        )
        .expect("transcript should drain")
        .expect("session limit should be terminal");

    assert!(failure.contains("substitutable resource limit"));
    let output = app
        .terminal()
        .output_records()
        .into_iter()
        .find(|record| record.kind == TerminalOutputKind::ProviderError)
        .expect("session limit should be projected as provider error");
    assert!(String::from_utf8_lossy(&output.bytes).contains("session limit"));
    assert!(!crate::transport::flow_control::prompt_completion_recorded(
        &app,
        run.id()
    ));
    assert!(app
        .providers()
        .get_run(run.id())
        .expect("run should remain available")
        .terminal_diagnostic()
        .is_some_and(|diagnostic| diagnostic.contains("session limit")));
    assert_eq!(
        app.attachments().list_session_attachment_ids(session.id()),
        vec![attachment.id().to_string()]
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn claude_transcript_drain_skips_content_before_active_prompt() {
    let mut cursor = ClaudeTranscriptCursor::default();
    let dir = std::env::temp_dir().join(format!(
        "chariox-claude-transcript-active-prompt-cutoff-test-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    let transcript = dir.join("session.jsonl");
    fs::write(
        &transcript,
        [
            serde_json::json!({
                "type": "assistant",
                "uuid": "old-assistant",
                "timestamp": "1970-01-01T00:00:01Z",
                "message": {
                    "id": "old-message",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "old response" }]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "assistant",
                "uuid": "current-assistant",
                "timestamp": "1970-01-01T00:00:03Z",
                "message": {
                    "id": "current-message",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "current response" }]
                }
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("fixture should write");

    let drain = drain_claude_transcript_file_since(
        &transcript.display().to_string(),
        &mut cursor,
        Some(2_000),
    );

    assert_eq!(drain.chunks.len(), 1);
    assert_eq!(drain.chunks[0].text, "current response");
    assert_eq!(drain.assistant_message_ids, vec!["current-message"]);
    let second = drain_claude_transcript_file_since(
        &transcript.display().to_string(),
        &mut cursor,
        Some(2_000),
    );
    assert!(second.chunks.is_empty());
    assert!(second.assistant_message_ids.is_empty());

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn claude_transcript_drain_ignores_internal_resume_pair_before_real_response() {
    let mut cursor = ClaudeTranscriptCursor::default();
    let dir = std::env::temp_dir().join(format!(
        "chariox-claude-transcript-resume-pair-test-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    let transcript = dir.join("session.jsonl");
    fs::write(
        &transcript,
        [
            serde_json::json!({
                "type": "user",
                "uuid": "synthetic-user",
                "isMeta": true,
                "sessionId": "claude-session-1",
                "message": {
                    "role": "user",
                    "content": [{ "type": "text", "text": "Continue from where you left off." }]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "assistant",
                "uuid": "synthetic-assistant",
                "parentUuid": "synthetic-user",
                "sessionId": "claude-session-1",
                "message": {
                    "id": "synthetic-message",
                    "model": "<synthetic>",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "No response requested." }]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "user",
                "uuid": "real-user",
                "sessionId": "claude-session-1",
                "message": {
                    "role": "user",
                    "content": [{ "type": "text", "text": "Explain authoritative completion." }]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "assistant",
                "uuid": "real-assistant",
                "sessionId": "claude-session-1",
                "message": {
                    "id": "real-message",
                    "model": "claude-opus-4-7",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "Only the real response settles the turn." }]
                }
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("fixture should write");

    let drain = drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);

    assert_eq!(drain.session_id.as_deref(), Some("claude-session-1"));
    assert_eq!(drain.model.as_deref(), Some("claude/claude-opus-4-7"));
    assert_eq!(drain.assistant_message_ids, vec!["real-message"]);
    assert_eq!(drain.chunks.len(), 1);
    assert_eq!(
        drain.chunks[0].text,
        "Only the real response settles the turn."
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn claude_transcript_drain_preserves_synthetic_api_errors() {
    let mut cursor = ClaudeTranscriptCursor::default();
    let dir = std::env::temp_dir().join(format!(
        "chariox-claude-transcript-api-error-test-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    let transcript = dir.join("session.jsonl");
    fs::write(
        &transcript,
        serde_json::json!({
            "type": "assistant",
            "uuid": "api-error-assistant",
            "isApiErrorMessage": true,
            "error": "authentication_failed",
            "sessionId": "claude-session-error",
            "message": {
                "id": "api-error-message",
                "model": "<synthetic>",
                "role": "assistant",
                "content": [{ "type": "text", "text": "Login expired · Please run /login" }]
            }
        })
        .to_string(),
    )
    .expect("fixture should write");

    let drain = drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);

    assert_eq!(drain.session_id.as_deref(), Some("claude-session-error"));
    assert_eq!(drain.model, None);
    assert_eq!(drain.assistant_message_ids, vec!["api-error-message"]);
    assert_eq!(drain.chunks.len(), 1);
    assert_eq!(drain.chunks[0].text, "Login expired · Please run /login");

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn claude_transcript_drain_exposes_queue_enqueues_without_rendering_them() {
    let mut cursor = ClaudeTranscriptCursor::default();
    let dir = std::env::temp_dir().join(format!(
        "chariox-claude-transcript-dedupe-test-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    let transcript = dir.join("session.jsonl");
    let assistant = serde_json::json!({
        "type": "assistant",
        "uuid": "assistant-1",
        "message": {
            "role": "assistant",
            "content": [{ "type": "text", "text": "once" }]
        }
    })
    .to_string();
    fs::write(
        &transcript,
        [
            serde_json::json!({
                "type": "queue-operation",
                "operation": "enqueue",
                "content": "Also report the package version."
            })
            .to_string(),
            assistant.clone(),
            assistant,
        ]
        .join("\n"),
    )
    .expect("fixture should write");

    let drain = drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);

    assert_eq!(drain.chunks.len(), 1);
    assert_eq!(drain.chunks[0].text, "once");
    assert_eq!(drain.assistant_message_ids, vec!["assistant-1"]);
    assert_eq!(
        drain.enqueued_prompts,
        vec!["Also report the package version."]
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn claude_transcript_completion_is_unique_per_assistant_message_id() {
    let mut cursor = ClaudeTranscriptCursor::default();
    let dir = std::env::temp_dir().join(format!(
        "chariox-claude-transcript-message-completion-dedupe-test-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&dir);
    let transcript = dir.join("session.jsonl");
    fs::write(
        &transcript,
        [
            serde_json::json!({
                "type": "assistant",
                "uuid": "assistant-entry-1",
                "message": {
                    "id": "msg_shared",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "first projection" }]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "assistant",
                "uuid": "assistant-entry-2",
                "message": {
                    "id": "msg_shared",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "second projection" }]
                }
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("fixture should write");

    let drain = drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);

    assert_eq!(drain.chunks.len(), 2);
    assert_eq!(drain.assistant_message_ids, vec!["msg_shared"]);

    let second = drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);
    assert!(second.assistant_message_ids.is_empty());

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn claude_headless_prompt_waiting_in_composer_detects_collapsed_paste() {
    assert!(claude_headless_prompt_waiting_in_composer(
        "paste again to expand - [Pasted text #2]",
        "expected prompt",
    ));
    assert!(claude_headless_prompt_waiting_in_composer(
        "\u{1b}[2m[Pasted text #1]\u{1b}[0m",
        "expected prompt",
    ));
    assert!(!claude_headless_prompt_waiting_in_composer(
        "CLAUDE-HEADLESS_WORKSPACE_LIVE_SYNC_TEXT_WRITE_DONE",
        "expected prompt",
    ));
}

#[test]
fn claude_headless_prompt_waiting_in_composer_detects_direct_prompt_text() {
    let prompt = "Use the native-claude-skill skill. Give the Chariox skill marker.";
    let rendered = format!("\u{1b}[?25l\u{1b}[H\r\u{1b}[37B{prompt}\u{1b}[40;1H\u{1b}[?25h");

    assert!(claude_headless_prompt_waiting_in_composer(
        &rendered, prompt
    ));
    assert!(!claude_headless_prompt_waiting_in_composer(
        &format!("{rendered} Gitifying... esc to interrupt"),
        prompt,
    ));
}

#[test]
fn claude_headless_composer_detects_current_cycle_footer_only_with_prompt_glyph() {
    assert!(claude_headless_composer_visible(
        "──────────────── ❯ ──────────────── ⏵⏵ mode (shift+tab to cycle)"
    ));
    assert!(!claude_headless_composer_visible(
        "The documentation says shift+tab to cycle through modes."
    ));
    assert!(!claude_headless_composer_visible(
        "❯ stale composer frame followed much later by reviewer prose: The documentation says shift+tab to cycle through modes."
    ));
}

#[test]
fn claude_headless_bypass_confirmation_detects_clipped_rendered_choice() {
    let rendered = "WARNING:Claude CoderunninginBypassPermissionsmode \
        Byproceeding,youacceptallresponsibilityforactionstaken \
        >1.No,exit 2. Yes, I accep Entertoconfirm-Esc to cancel";

    assert!(claude_headless_bypass_confirmation_visible(rendered));
    assert!(!claude_headless_bypass_confirmation_visible(
        "Bypass permissions on - for shortcuts",
    ));
}

#[test]
fn claude_headless_bypass_selection_marker_is_distinct_from_prompt_state() {
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-bypass-selection-test-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let context_file = root.join("hidden-context.txt");
    fs::write(&context_file, "").expect("context file should be created");
    let context_file = context_file.display().to_string();

    assert!(!claude_headless_bypass_selection_pending(&context_file));
    write_claude_headless_bypass_selection_marker(&context_file);
    assert!(claude_headless_bypass_selection_pending(&context_file));
    write_claude_headless_startup_wait_marker(&context_file);
    assert!(!claude_headless_bypass_selection_pending(&context_file));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn queued_claude_failed_request_note_reaches_hook_context_and_waits_for_acceptance() {
    queued_claude_failed_request_note_impl(false, false);
}

#[test]
fn managed_hook_delivery_receipt_fences_headless_acknowledgement() {
    queued_claude_failed_request_note_impl(true, false);
}

#[test]
fn managed_steering_delivery_receipt_follows_target_active_prompt() {
    queued_claude_failed_request_note_impl(true, true);
}

fn queued_claude_failed_request_note_impl(receipts: bool, steering: bool) {
    let worktree = crate::test_support::TestWorktree::new("claude-queued-failed-note");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "failed-note-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let root = std::env::temp_dir().join(format!(
        "chariox-failed-note-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).unwrap();
    let context_file = root.join("hidden-context.txt");
    let events_file = root.join("events.jsonl");
    fs::write(&context_file, "").unwrap();
    fs::write(&events_file, "").unwrap();
    let mut run = startup_readiness_run(
        session.id(),
        agent.id(),
        "failed-note-claude-run",
        &context_file,
        &events_file,
        "cat >/dev/null".to_string(),
    );
    run.mark_running();
    let mut ready_run = serde_json::to_value(&run).unwrap();
    ready_run["started_at_ms"] = serde_json::json!(unix_epoch_ms().saturating_sub(5_000));
    if receipts {
        ready_run["pty_env"]["CHARIOX_CLAUDE_NATIVE_CONTEXT_RECEIPTS"] = serde_json::json!("1");
    }
    run = serde_json::from_value(ready_run).unwrap();
    app.pty.spawn_for_run(&run).unwrap();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .unwrap();
    let prompt = match app
        .record_native_prompt_started_with_attachments(
            session.id(),
            attachment.id(),
            attachment.id(),
            agent.id(),
            "next queued request",
            Vec::new(),
        )
        .unwrap()
    {
        crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
        other => panic!("unexpected prompt: {other:?}"),
    };
    app.agents
        .record_failed_request_durably(
            &app.durable_state_store(),
            agent.id(),
            crate::agent::FailedRequest::new(
                "earlier-failure",
                "earlier request",
                "rejected".to_string(),
            ),
        )
        .unwrap();
    let context_path = context_file.display().to_string();
    if steering {
        // MP-08 / MP-10 / MP-11: dispatch identity differs from its still-active target.
        let steering_id = "native-steering-request";
        write_claude_native_marker(&context_path, &format!("injected:{steering_id}"));
        fs::write(
            format!("{context_path}.context-delivery"),
            serde_json::json!({
                "request_id":"steering-receipt", "run_id":run.id(), "prompt_id":steering_id,
                "active_prompt_id":prompt.id(), "managed":true,
            })
            .to_string(),
        )
        .unwrap();
        ProviderOutputClaudeNativeBridge::new(&mut app)
            .accept_context_delivery(
                session.id(),
                agent.id(),
                run.id(),
                &context_path,
                &serde_json::json!({"hook_context_request_id":"steering-receipt"}),
            )
            .unwrap();
        let marker = claude_native_marker(&context_path);
        let active = app
            .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
            .unwrap()
            .unwrap();
        let failed = app
            .agents
            .get_agent(agent.id())
            .unwrap()
            .failed_requests()
            .len();
        app.pty.remove_process(run.id()).unwrap();
        fs::remove_dir_all(root).unwrap();
        assert_eq!(
            marker.as_deref(),
            Some("accepted:native-steering-request"),
            "steering receipt could not acknowledge its current active target"
        );
        assert_eq!(
            active.id(),
            prompt.id(),
            "steering must not replace the active turn"
        );
        assert_eq!(
            failed, 1,
            "steering delivery must retain active-turn failure context"
        );
        return;
    }
    write_claude_native_marker(&context_path, &format!("post-stop-ready:{}", prompt.id()));
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process(session.id(), run.id(), &run, None)
        .unwrap();
    let hidden = fs::read_to_string(&context_file).unwrap();
    assert!(
        hidden.contains("Your previous request (\"earlier request\") failed"),
        "{hidden}"
    );
    assert_eq!(
        app.agents
            .get_agent(agent.id())
            .unwrap()
            .failed_requests()
            .len(),
        1,
        "writing hook context does not prove provider acceptance"
    );
    crate::app::prompt_lifecycle::ProviderPromptDispatcher::new(&mut app)
        .dispatch_prompt_to_provider(
            session.id(),
            run.id(),
            prompt.id(),
            attachment.id(),
            prompt.prompt(),
            prompt.hidden_system_context(),
            prompt.attachments(),
        )
        .unwrap();
    assert_eq!(
        app.agents
            .get_agent(agent.id())
            .unwrap()
            .failed_requests()
            .len(),
        1,
        "app dispatch must retain context until the hook accepts it"
    );
    write_claude_native_marker(&context_path, &format!("injected:{}", prompt.id()));
    fs::write(
        &events_file,
        serde_json::json!({
            "hook_event_name": "UserPromptSubmit", "prompt": prompt.prompt(),
            "hook_context_request_id": if receipts { Some("managed-current") } else { None },
        })
        .to_string(),
    )
    .unwrap();
    if receipts {
        acknowledge_claude_headless_dispatch_from_hook_events(
            &context_path,
            events_file.to_str().unwrap(),
            prompt.id(),
            true,
        );
        assert_eq!(
            claude_native_marker(&context_path).as_deref(),
            Some(format!("injected:{}", prompt.id()).as_str()),
            "headless shortcut consumed context before hook delivery"
        );
    }
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process(session.id(), run.id(), &run, None)
        .unwrap();
    if receipts {
        assert_eq!(
            app.agents
                .get_agent(agent.id())
                .unwrap()
                .failed_requests()
                .len(),
            1,
            "managed response creation must not acknowledge delivery"
        );
        assert!(root
            .join("hook-context-responses/managed-current.txt")
            .exists());
        fs::write(&events_file, serde_json::json!({
            "hook_event_name":"CharioxContextDelivered", "hook_context_request_id":"managed-current",
        }).to_string()).unwrap();
        ProviderOutputClaudeNativeBridge::new(&mut app)
            .process(session.id(), run.id(), &run, None)
            .unwrap();
    }
    assert!(app
        .agents
        .get_agent(agent.id())
        .unwrap()
        .failed_requests()
        .is_empty());
    assert!(
        claude_native_marker(&context_path).as_deref()
            == Some(format!("accepted:{}", prompt.id()).as_str())
    );
    let _ = fs::remove_dir_all(root);
}

#[derive(Clone)]
struct RefusedPermissionBridge;
impl ProviderNativeInteractionBridge for RefusedPermissionBridge {
    fn request_blocking(
        &self,
        _session_id: &str,
        _interaction: RuntimeInteraction,
    ) -> Result<crate::provider::ProviderNativeInteractionResolution, DaemonError> {
        Ok(crate::provider::ProviderNativeInteractionResolution {
            status: "timed_out".into(),
            choice_id: None,
            reply: None,
        })
    }
}

#[test]
fn approval_lifetime_refused_claude_dialog_receives_deny_while_displayed() {
    for startup in [false, true] {
        let worktree = crate::test_support::TestWorktree::new("refused-claude-dialog");
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
            .unwrap();
        let context = worktree.path().join("hidden-context.txt");
        let events = worktree.path().join("events.jsonl");
        fs::write(&context, "").unwrap();
        fs::write(&events, "").unwrap();
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude-headless",
            "default",
            "claude-sonnet",
        )
        .with_agent_id(agent.id())
        .with_client_interface(crate::provider::ProviderClientInterface::NativeTui)
        .with_permission_level(crate::provider::AgentPermissionLevel::Required);
        let run = RuntimeProviderRun::new(
            "refused-run",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "recorded-claude-dialog".into(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::from([
                    (
                        "CHARIOX_CLAUDE_NATIVE_CONTEXT".into(),
                        context.display().to_string(),
                    ),
                    (
                        "CHARIOX_CLAUDE_NATIVE_EVENTS".into(),
                        events.display().to_string(),
                    ),
                ]),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        let context = context.display().to_string();
        let mut native = ProviderOutputClaudeNativeBridge::new(&mut app);
        if startup {
            native
                .request_claude_workspace_trust(
                    session.id(),
                    run.id(),
                    &run,
                    &context,
                    std::sync::Arc::new(RefusedPermissionBridge),
                    "Quick safety check\nDo you trust this folder?\n1. No, exit\n2. Yes, I trust this folder",
                )
                .unwrap();
        } else {
            native
                .process_terminal_output(
                    session.id(),
                    run.id(),
                    &run,
                    Some(std::sync::Arc::new(RefusedPermissionBridge)),
                    "Bash command\necho test\nDo you want to proceed?\n1. Yes\n3. No",
                )
                .unwrap();
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let inputs = take_claude_permission_inputs(&context);
            if !inputs.is_empty() {
                assert_eq!(
                    inputs,
                    vec![vec![0x03]],
                    "a refused displayed dialog must receive Deny"
                );
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "refused dialog was left unanswered"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

#[test]
fn native_claude_cancellation_clears_only_its_unacknowledged_composer() {
    for (marker, expected_input) in [
        ("injected:cancel-native", b"\x03\x15".as_slice()),
        ("accepted:cancel-native", b"\x03".as_slice()),
        ("injected:another-prompt", b"\x03".as_slice()),
    ] {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "native-cancel-workspace",
                "native-cancel-worktree",
            ))
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "native-cancel-client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let root = std::env::temp_dir().join(format!(
            "chariox-native-cancel-{}-{}",
            std::process::id(),
            timestamp_millis()
        ));
        fs::create_dir_all(&root).unwrap();
        let context_file = root.join("hidden-context.txt");
        fs::write(&context_file, "").unwrap();
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude",
            "default",
            "sonnet",
        )
        .with_agent_id(agent.id())
        .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
        let mut run = RuntimeProviderRun::new(
            "native-cancel-run",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "native-cancel-test".to_string(),
                pty_target: Some("native-cancel-pty".to_string()),
                pty_program: Some("/bin/sh".to_string()),
                pty_args: vec!["-c".to_string(), "trap '' INT; exec cat".to_string()],
                pty_env: std::collections::BTreeMap::from([(
                    "CHARIOX_CLAUDE_NATIVE_CONTEXT".to_string(),
                    context_file.display().to_string(),
                )]),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.mark_running();
        app.providers_mut().insert_run_for_test(run.clone());
        app.sessions
            .set_active_provider_run(session.id(), Some(run.id().to_string()))
            .unwrap();
        app.pty.spawn_for_run(&run).unwrap();
        crate::app::provider_runtime::ProviderProcessTracker::new(&mut app)
            .register_managed_run(&run)
            .unwrap();
        let context = context_file.to_str().unwrap();
        let mut prompt = crate::session::PromptQueueItem::new(
            "cancel-native",
            attachment.id(),
            agent.id(),
            "managed audit prompt",
            crate::session::PromptStatus::Dispatching,
        );
        prompt.set_durable_delivery(
            crate::session::DurablePromptDeliveryPhase::Dispatching,
            Some(run.id().to_string()),
            None,
        );
        app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .unwrap();
        let prompt_id = app
            .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
            .unwrap()
            .unwrap()
            .id()
            .to_string();
        let marker = match marker {
            "injected:cancel-native" => format!("injected:{prompt_id}"),
            "accepted:cancel-native" => format!("accepted:{prompt_id}"),
            foreign => foreign.to_string(),
        };
        write_claude_native_marker(context, &marker);
        write_claude_headless_submit_retry(
            context,
            &prompt_id,
            0,
            unix_epoch_ms(),
            "managed audit prompt",
        );
        app.cancel_active_prompt(session.id(), attachment.id())
            .unwrap();
        assert_eq!(
            app.terminal().input_records().last().unwrap().bytes,
            expected_input,
            "marker={marker}"
        );
        assert!(
            app.pty.has_process(run.id()),
            "native cancellation must preserve the user's provider TUI"
        );
        crate::app::provider_runtime::ProviderProcessTracker::new(&mut app)
            .remove_run(run.id())
            .unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn native_auto_compaction_hook_keeps_current_user_turn() {
    let worktree = crate::test_support::TestWorktree::new("claude-native-auto-compact");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "compact-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let root = std::env::temp_dir().join(format!(
        "chariox-native-auto-compact-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).unwrap();
    let context_file = root.join("hidden-context.txt");
    let events_file = root.join("events.jsonl");
    fs::write(&context_file, "").unwrap();
    fs::write(&events_file, serde_json::json!({
        "hook_event_name": "UserPromptSubmit",
        "prompt": "This session is being continued from a previous conversation that ran out of context. The summary below covers the earlier portion of the conversation. Summary: Preserve the audit and answer the current memory probe."
    }).to_string()).unwrap();
    let mut run = startup_readiness_run(
        session.id(),
        agent.id(),
        "run-auto-compact",
        &context_file,
        &events_file,
        "cat >/dev/null".to_string(),
    );
    run.mark_running();
    app.pty.spawn_for_run(&run).unwrap();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .unwrap();
    let prompt = match app
        .record_native_prompt_started_with_attachments(
            session.id(),
            attachment.id(),
            attachment.id(),
            agent.id(),
            "Recall the ten accepted audit facts without tools.",
            Vec::new(),
        )
        .unwrap()
    {
        crate::session::PromptSubmissionOutcome::Started { prompt } => prompt,
        outcome => panic!("unexpected prompt outcome: {outcome:?}"),
    };
    write_claude_native_marker(
        &context_file.display().to_string(),
        &format!("native:{}", prompt.id()),
    );
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process(session.id(), run.id(), &run, None)
        .unwrap();
    let active_id = app
        .sessions()
        .get_session(session.id())
        .unwrap()
        .active_prompt_for_agent(agent.id())
        .map(|prompt| prompt.id().to_string());
    let (_, queued) = app.prompt_state_owner().state_parts(
        &app.sessions().get_session(session.id()).unwrap(),
        agent.id(),
    );
    app.pty.remove_process(run.id()).unwrap();
    fs::remove_dir_all(&root).unwrap();
    assert_eq!(
        active_id.as_deref(),
        Some(prompt.id()),
        "provider compaction must not replace the user's active turn with a generated summary"
    );
    assert!(
        queued.is_empty(),
        "provider compaction must not enqueue a generated user prompt"
    );
}

// MP-08 / MP-10 / MP-11: fit after the worker adds its normal origin context.
#[test]
fn review976_recreated_replay_fits_complete_claude_worker_context() {
    assert_recreated_replay_fits_worker("claude");
    assert_recreated_replay_fits_worker("claude-headless");
}

fn assert_recreated_replay_fits_worker(provider: &str) {
    let worktree = crate::test_support::TestWorktree::new("claude-replay-fit");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-replay-fit-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).unwrap();
    let context_file = root.join("hidden-context.txt");
    fs::write(&context_file, "").unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        provider,
        "default",
        "sonnet",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let run = RuntimeProviderRun::new(
        "claude-replay-fit",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "fixture".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    if provider == "claude-headless" {
        // Let the native composer startup grace expire; no provider process is started here.
        std::thread::sleep(std::time::Duration::from_millis(4_100));
    }
    let required = "x".repeat(if provider == "claude-headless" {
        21_000
    } else {
        45_000
    });
    let open = "<chariox_context_handoff>\nChariox reconstructed this bounded background from home history after worker lease recreation. The active user request is supplied separately.\n\n";
    let close = "\n</chariox_context_handoff>";
    let body_room = crate::provider::claude_native_hidden_context_room(&required).min(24_000)
        - open.len()
        - close.len();
    let latest = "Latest turn:\nInterrupted attempt: preserve the pending tool and next step";
    let body = format!(
        "Prior turns:\n{}\n\n{latest}",
        "🦀".repeat((body_room - latest.len() - 17) / 4)
    );
    let hidden = format!("{required}\n\n{open}{body}{close}");
    crate::provider::ensure_claude_native_hidden_context_fits(run.id(), &hidden).unwrap();
    let injected = ProviderOutputClaudeNativeBridge::new(&mut app).inject_prompt(
        session.id(),
        run.id(),
        agent.id(),
        context_file.to_str().unwrap(),
        &run,
        &ClaudeNativePromptInjection {
            id: "recreated-prompt",
            origin_prompt_id: "normal-origin-prompt",
            prompt: "",
            hidden_system_context: &hidden,
            context_handoff: &no_context_handoff,
            attachments: &[],
        },
    );
    // MP-08 / MP-10 / MP-11: #919 reconstructs staged inputs again at hook ACK.
    // It must preserve #976's complete-worker-context fitting at that boundary.
    let rebuilt = ProviderOutputClaudeNativeBridge::new(&mut app)
        .managed_base_context(
            session.id(),
            agent.id(),
            context_file.to_str().unwrap(),
            &run,
            &ClaudeNativePromptInjection {
                id: "recreated-prompt",
                origin_prompt_id: "normal-origin-prompt",
                prompt: "",
                hidden_system_context: &hidden,
                context_handoff: &no_context_handoff,
                attachments: &[],
            },
        )
        .unwrap();
    crate::provider::ensure_claude_native_hidden_context_fits(run.id(), &rebuilt)
        .expect("hook reconstruction must fit the complete worker context too");
    assert!(rebuilt.contains(&required));
    assert!(rebuilt.contains(latest));
    let delivered = fs::read_to_string(&context_file).unwrap();
    fs::remove_dir_all(&root).unwrap();
    injected.expect("optional replay must fit after normal worker context is added");
    assert!(delivered.contains(&required));
    assert!(delivered.contains("normal-origin-prompt"));
    assert!(
        delivered.contains(latest),
        "worker trimming must retain the latest interrupted attempt"
    );
    assert!(
        delivered.contains("[truncated]"),
        "worker trimming must disclose omitted prior turns"
    );
    crate::provider::ensure_claude_native_hidden_context_fits(run.id(), &delivered).unwrap();
    assert!(
        delivered.contains("<chariox_context_handoff>")
            == delivered.contains("</chariox_context_handoff>")
    );
}

// MP-08 / MP-10 / MP-11: mandatory worker context cannot consume a replay with no latest body.
#[test]
fn review976r2_claude_rejects_replay_with_only_a_partial_latest_header() {
    let open = crate::prompt_assembly::RECREATED_LEASE_CONTEXT_OPEN;
    let close = crate::prompt_assembly::RECREATED_LEASE_CONTEXT_CLOSE;
    let capacity = 8 * (6_000 - 3);
    let required = "x".repeat(capacity - open.len() - close.len() - 16);
    let context = format!("{required}{open}Latest turn:\n{}{close}", "🦀".repeat(100));
    assert!(
        crate::provider::fit_claude_native_hidden_context("tiny-replay-budget", context).is_err()
    );
}

fn note_attachment(quote: &str, comment: &str) -> PromptAttachment {
    use base64::Engine as _;
    let json = serde_json::json!({ "quote": quote, "comment": comment }).to_string();
    PromptAttachment::new(
        "chariox-terminal://note/1",
        crate::prompt_notes::NOTE_ATTACHMENT_MIME,
        Some("Note".into()),
    )
    .with_contents_base64(base64::engine::general_purpose::STANDARD.encode(json))
}

#[test]
fn claude_types_the_prompt_and_never_the_note_text() {
    let attachments = vec![note_attachment("secret quote", "secret comment")];
    let typed = claude_typed_prompt(
        &ClaudeNativePromptInjection {
            id: "p",
            origin_prompt_id: "p",
            prompt: "fix it",
            hidden_system_context: "",
            attachments: &attachments,
            context_handoff: &|_| String::new(),
        },
        "/nonexistent/context",
    );
    assert_eq!(typed, "fix it");
}

#[test]
fn notes_reach_claude_hidden_context_once_and_fit_the_bridge_limit() {
    let attachments = vec![note_attachment(&"q".repeat(12_000), &"c".repeat(12_000))];
    let (files, notes) = crate::prompt_notes::split_for_provider(attachments, None);
    assert!(files.is_empty());
    let context = join_claude_context([
        format_claude_attachment_context(&files, "/nonexistent/context"),
        notes.unwrap_or_default(),
    ]);
    assert_eq!(context.matches(&"q".repeat(12_000)).count(), 1);
    assert!(!context.contains("Attachment 1"));
    assert!(context.len() < 48_000, "{}", context.len());
}

mod workspace_scope;
