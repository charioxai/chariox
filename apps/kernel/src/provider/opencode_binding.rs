use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::error::DaemonError;
use rand::distributions::{Alphanumeric, DistString};

use super::{
    opencode_client::OpenCodeConfiguredDefaults, workspace_write_fence_active, OpenCodeClient,
    OpenCodeMessage, ProviderResumeState, RuntimeProviderRun,
};
use crate::provider::opencode_runtime::{drain_opencode_events, OpenCodeRuntimeState};
use crate::terminal::TerminalOutputKind;

const OPENCODE_EVENT_SUBSCRIBE_TIMEOUT: Duration = Duration::from_secs(5);
const OPENCODE_EVENT_SUBSCRIBE_RETRY_INTERVAL: Duration = Duration::from_millis(100);
const OPENCODE_SESSION_CREATE_TIMEOUT: Duration = Duration::from_secs(5);
const OPENCODE_SESSION_CREATE_RETRY_INTERVAL: Duration = Duration::from_millis(100);
const OPENCODE_MCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(180);
const OPENCODE_MCP_CONNECT_RETRY_INTERVAL: Duration = Duration::from_secs(1);
const OPENCODE_ABORT_SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(10);
const OPENCODE_ABORT_SETTLEMENT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const OPENCODE_ABORT_SETTLEMENT_QUIET_PERIOD: Duration = Duration::from_millis(200);
const OPENCODE_UTILITY_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Default)]
pub(crate) struct OpenCodeRunSelection {
    pub model: Option<String>,
    pub variant: Option<String>,
}

pub(crate) struct OpenCodeRuntimeBinding {
    pub state: OpenCodeRuntimeState,
    pub selection: OpenCodeRunSelection,
    pub resume_state: ProviderResumeState,
}

pub(crate) fn initialize_opencode_runtime(
    run: &RuntimeProviderRun,
) -> Result<OpenCodeRuntimeBinding, DaemonError> {
    let base_url = run
        .structured_endpoint()
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "opencode_endpoint_missing",
            message: "opencode run did not expose a structured endpoint".to_string(),
        })?
        .to_string();
    let client = OpenCodeClient::new(run.id(), &base_url)?;
    crate::logging::info_with_fields(
        "daemon.provider.opencode",
        "waiting for opencode health",
        serde_json::json!({
            "provider_run_id": run.id(),
            "base_url": base_url.clone(),
        }),
    );
    client.wait_until_healthy(Duration::from_secs(30))?;
    crate::logging::info_with_fields(
        "daemon.provider.opencode",
        "opencode became healthy",
        serde_json::json!({
            "provider_run_id": run.id(),
            "base_url": base_url.clone(),
        }),
    );
    ensure_configured_mcp_servers_connected(run, &client)?;

    let selection = resolve_initial_selection(run, &client)?;

    let allow_native_writes = opencode_workspace_live_sync_native_writes_allowed(run);
    let session_permission = if run.requires_workspace_live_sync() {
        Some(opencode_workspace_live_sync_permission_rules(
            allow_native_writes,
            run.permission_level(),
        ))
    } else {
        Some(opencode_permission_rules(run.permission_level()))
    };
    let resumable_session_id = run.resume_state().opencode_session_id().map(str::to_string);
    let (session_id, preexisting_messages) = match resumable_session_id {
        Some(previous_session_id) => match client.snapshot(&previous_session_id) {
            Ok(snapshot) => {
                crate::logging::info_with_fields(
                    "daemon.provider.opencode",
                    "reusing opencode session",
                    serde_json::json!({
                        "provider_run_id": run.id(),
                        "provider_session_id": previous_session_id.clone(),
                    }),
                );
                (previous_session_id, snapshot.messages)
            }
            Err(_) => {
                crate::logging::warn_with_fields(
                    "daemon.provider.opencode",
                    "opencode session resume failed; creating a new session",
                    serde_json::json!({
                        "provider_run_id": run.id(),
                        "provider_session_id": previous_session_id,
                    }),
                );
                let session_id = client.create_session_with_retry(
                    session_permission.clone(),
                    OPENCODE_SESSION_CREATE_TIMEOUT,
                    OPENCODE_SESSION_CREATE_RETRY_INTERVAL,
                )?;
                crate::logging::info_with_fields(
                    "daemon.provider.opencode",
                    "created opencode session",
                    serde_json::json!({
                        "provider_run_id": run.id(),
                        "provider_session_id": session_id.clone(),
                    }),
                );
                (session_id, Vec::new())
            }
        },
        None => {
            let session_id = client.create_session_with_retry(
                session_permission.clone(),
                OPENCODE_SESSION_CREATE_TIMEOUT,
                OPENCODE_SESSION_CREATE_RETRY_INTERVAL,
            )?;
            crate::logging::info_with_fields(
                "daemon.provider.opencode",
                "created opencode session",
                serde_json::json!({
                    "provider_run_id": run.id(),
                    "provider_session_id": session_id.clone(),
                }),
            );
            (session_id, Vec::new())
        }
    };
    let event_subscription = client.subscribe_events_with_retry(
        OPENCODE_EVENT_SUBSCRIBE_TIMEOUT,
        OPENCODE_EVENT_SUBSCRIBE_RETRY_INTERVAL,
    )?;
    crate::logging::info_with_fields(
        "daemon.provider.opencode",
        "subscribed to opencode events",
        serde_json::json!({
            "provider_run_id": run.id(),
        }),
    );

    let mut state = OpenCodeRuntimeState::new(base_url, session_id.clone(), event_subscription);
    state.baseline_existing_messages(&preexisting_messages);

    Ok(OpenCodeRuntimeBinding {
        state,
        selection,
        resume_state: ProviderResumeState::from_opencode_session_id(session_id),
    })
}

fn ensure_configured_mcp_servers_connected(
    run: &RuntimeProviderRun,
    client: &OpenCodeClient,
) -> Result<(), DaemonError> {
    let mut names = Vec::new();
    if run.runtime_mcp_server_url().is_some() {
        names.push("chariox".to_string());
    }
    names.extend(run.mcp_servers().iter().map(|server| server.name.clone()));
    names.sort();
    names.dedup();
    for name in names {
        client.connect_mcp_server_with_retry(
            &name,
            OPENCODE_MCP_CONNECT_TIMEOUT,
            OPENCODE_MCP_CONNECT_RETRY_INTERVAL,
        )?;
        client.wait_until_mcp_server_connected(
            &name,
            OPENCODE_MCP_CONNECT_TIMEOUT,
            OPENCODE_MCP_CONNECT_RETRY_INTERVAL,
        )?;
        crate::logging::info_with_fields(
            "daemon.provider.opencode",
            "connected opencode MCP server",
            serde_json::json!({
                "provider_run_id": run.id(),
                "mcp_server": name,
            }),
        );
    }
    Ok(())
}

fn opencode_workspace_live_sync_permission_rules(
    allow_native_writes: bool,
    permission_level: crate::provider::AgentPermissionLevel,
) -> serde_json::Value {
    let native_action = opencode_permission_action(permission_level);
    let native_write_action = if allow_native_writes {
        native_action
    } else {
        "ask"
    };
    let rules = vec![
        serde_json::json!({
            "permission": "edit",
            "pattern": "*",
            "action": native_write_action
        }),
        serde_json::json!({
            "permission": "write",
            "pattern": "*",
            "action": native_write_action
        }),
        serde_json::json!({
            "permission": "multiedit",
            "pattern": "*",
            "action": native_write_action
        }),
        serde_json::json!({
            "permission": "apply_patch",
            "pattern": "*",
            "action": native_write_action
        }),
        serde_json::json!({
            "permission": "external_directory",
            "pattern": "*",
            "action": native_write_action
        }),
        serde_json::json!({
            "permission": "bash",
            "pattern": "*",
            "action": native_action
        }),
        serde_json::json!({
            "permission": "doom_loop",
            "pattern": "*",
            "action": native_action
        }),
        serde_json::json!({
            "permission": "task",
            "pattern": "*",
            "action": "deny"
        }),
    ];
    serde_json::Value::Array(rules)
}

fn opencode_workspace_live_sync_native_writes_allowed(run: &RuntimeProviderRun) -> bool {
    run.tracks_workspace_live_sync() || workspace_write_fence_active(run)
}

fn opencode_prompt_should_disable_native_writes(_run: &RuntimeProviderRun) -> bool {
    false
}

fn opencode_prompt_should_allow_native_bash(run: &RuntimeProviderRun) -> bool {
    run.requires_workspace_live_sync() || workspace_write_fence_active(run)
}

fn opencode_permission_rules(
    permission_level: crate::provider::AgentPermissionLevel,
) -> serde_json::Value {
    let action = opencode_permission_action(permission_level);
    serde_json::json!([
        {
            "permission": "edit",
            "pattern": "*",
            "action": action
        },
        {
            "permission": "write",
            "pattern": "*",
            "action": action
        },
        {
            "permission": "multiedit",
            "pattern": "*",
            "action": action
        },
        {
            "permission": "apply_patch",
            "pattern": "*",
            "action": action
        },
        {
            "permission": "external_directory",
            "pattern": "*",
            "action": action
        },
        {
            "permission": "bash",
            "pattern": "*",
            "action": action
        },
        {
            "permission": "doom_loop",
            "pattern": "*",
            "action": action
        },
        {
            "permission": "task",
            "pattern": "*",
            "action": action
        }
    ])
}

fn opencode_permission_action(
    permission_level: crate::provider::AgentPermissionLevel,
) -> &'static str {
    match permission_level {
        crate::provider::AgentPermissionLevel::Required => "ask",
        crate::provider::AgentPermissionLevel::Yolo => "allow",
    }
}

pub(super) fn sync_opencode_run_selection_for_session(
    provider_run_id: &str,
    base_url: &str,
    session_id: &str,
    requested_model: &str,
    requested_variant: Option<&str>,
) -> Result<OpenCodeRunSelection, DaemonError> {
    let client = OpenCodeClient::new(provider_run_id, base_url)?;
    let defaults = client.configured_defaults()?;
    let messages = client.messages(session_id)?;

    Ok(resolve_sync_selection(
        &messages,
        defaults,
        requested_model,
        requested_variant,
    ))
}

fn resolve_sync_selection(
    messages: &[OpenCodeMessage],
    defaults: OpenCodeConfiguredDefaults,
    requested_model: &str,
    requested_variant: Option<&str>,
) -> OpenCodeRunSelection {
    OpenCodeRunSelection {
        model: messages
            .iter()
            .rev()
            .find_map(|message| message.info.resolved_model())
            .or_else(|| {
                (requested_model == "default")
                    .then_some(defaults.model)
                    .flatten()
            }),
        variant: messages
            .iter()
            .rev()
            .find_map(|message| message.info.resolved_variant())
            .or_else(|| {
                requested_variant
                    .is_none()
                    .then_some(defaults.variant)
                    .flatten()
            }),
    }
}

pub(super) fn abort_opencode_session(
    run: &RuntimeProviderRun,
    state: &mut OpenCodeRuntimeState,
) -> Result<(), DaemonError> {
    let client = OpenCodeClient::new(run.id(), state.base_url())?;
    client.abort_session(state.session_id())?;
    let deadline = Instant::now() + OPENCODE_ABORT_SETTLEMENT_TIMEOUT;
    let mut idle_signature = None;
    let mut idle_since = None;
    loop {
        match client.snapshot(state.session_id()) {
            Ok(snapshot) if snapshot.status == "idle" => {
                let signature = snapshot
                    .messages
                    .iter()
                    .map(|message| (message.info.id.clone(), message.info.time.completed))
                    .collect::<Vec<_>>();
                if idle_signature.as_ref() != Some(&signature) {
                    idle_signature = Some(signature);
                    idle_since = Some(Instant::now());
                } else if idle_since
                    .is_some_and(|since| since.elapsed() >= OPENCODE_ABORT_SETTLEMENT_QUIET_PERIOD)
                {
                    let allow_native_writes =
                        opencode_workspace_live_sync_native_writes_allowed(run);
                    let session_permission = if run.requires_workspace_live_sync() {
                        Some(opencode_workspace_live_sync_permission_rules(
                            allow_native_writes,
                            run.permission_level(),
                        ))
                    } else {
                        Some(opencode_permission_rules(run.permission_level()))
                    };
                    let session_id = client.create_session_with_retry(
                        session_permission,
                        OPENCODE_SESSION_CREATE_TIMEOUT,
                        OPENCODE_SESSION_CREATE_RETRY_INTERVAL,
                    )?;
                    state.switch_session_after_abort(session_id);
                    return Ok(());
                }
                std::thread::sleep(OPENCODE_ABORT_SETTLEMENT_POLL_INTERVAL);
            }
            Ok(_) if Instant::now() < deadline => {
                idle_signature = None;
                idle_since = None;
                std::thread::sleep(OPENCODE_ABORT_SETTLEMENT_POLL_INTERVAL);
            }
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(OPENCODE_ABORT_SETTLEMENT_POLL_INTERVAL);
            }
            Ok(snapshot) => {
                return Err(DaemonError::ProviderProtocol {
                    provider_run_id: run.id().to_string(),
                    operation: "opencode_abort_settlement_timeout",
                    message: format!(
                        "OpenCode session `{}` remained `{}` after abort",
                        state.session_id(),
                        snapshot.status,
                    ),
                });
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use serde_json::json;

    use super::{
        next_opencode_message_id, opencode_permission_rules,
        opencode_prompt_should_allow_native_bash, opencode_prompt_should_disable_native_writes,
        opencode_workspace_live_sync_native_writes_allowed,
        opencode_workspace_live_sync_permission_rules, resolve_sync_selection,
        submit_opencode_prompt, OpenCodeConfiguredDefaults, OpenCodeMessage, OpenCodeRuntimeState,
    };
    use crate::provider::{
        AgentEndpointMode, LaunchProviderRequest, OpenCodeEventSubscription, ProviderLaunchResult,
        RuntimeProviderRun,
    };

    fn message(provider_id: &str, model_id: &str, variant: &str) -> OpenCodeMessage {
        serde_json::from_value(json!({
            "info": {
                "id": "msg-1",
                "sessionID": "session-1",
                "role": "assistant",
                "providerID": provider_id,
                "modelID": model_id,
                "variant": variant
            },
            "parts": []
        }))
        .expect("message should parse")
    }

    #[test]
    fn normal_permission_rules_cover_active_write_permissions() {
        assert_eq!(
            opencode_permission_rules(crate::provider::AgentPermissionLevel::Yolo),
            json!([
                {
                    "permission": "edit",
                    "pattern": "*",
                    "action": "allow"
                },
                {
                    "permission": "write",
                    "pattern": "*",
                    "action": "allow"
                },
                {
                    "permission": "multiedit",
                    "pattern": "*",
                    "action": "allow"
                },
                {
                    "permission": "apply_patch",
                    "pattern": "*",
                    "action": "allow"
                },
                {
                    "permission": "external_directory",
                    "pattern": "*",
                    "action": "allow"
                },
                {
                    "permission": "bash",
                    "pattern": "*",
                    "action": "allow"
                },
                {
                    "permission": "doom_loop",
                    "pattern": "*",
                    "action": "allow"
                },
                {
                    "permission": "task",
                    "pattern": "*",
                    "action": "allow"
                }
            ])
        );
    }

    #[test]
    fn workspace_live_sync_permission_rules_gate_native_writes_without_fence() {
        assert_eq!(
            opencode_workspace_live_sync_permission_rules(
                false,
                crate::provider::AgentPermissionLevel::Yolo,
            ),
            json!([
                {
                    "permission": "edit",
                    "pattern": "*",
                    "action": "ask"
                },
                {
                    "permission": "write",
                    "pattern": "*",
                    "action": "ask"
                },
                {
                    "permission": "multiedit",
                    "pattern": "*",
                    "action": "ask"
                },
                {
                    "permission": "apply_patch",
                    "pattern": "*",
                    "action": "ask"
                },
                {
                    "permission": "external_directory",
                    "pattern": "*",
                    "action": "ask"
                },
                {
                    "permission": "bash",
                    "pattern": "*",
                    "action": "allow"
                },
                {
                    "permission": "doom_loop",
                    "pattern": "*",
                    "action": "allow"
                },
                {
                    "permission": "task",
                    "pattern": "*",
                    "action": "deny"
                }
            ])
        );
    }

    #[test]
    fn workspace_live_sync_permission_rules_allow_native_writes_when_workspace_is_fenced() {
        assert_eq!(
            opencode_workspace_live_sync_permission_rules(
                true,
                crate::provider::AgentPermissionLevel::Yolo,
            ),
            json!([
                {
                    "permission": "edit",
                    "pattern": "*",
                    "action": "allow"
                },
                {
                    "permission": "write",
                    "pattern": "*",
                    "action": "allow"
                },
                {
                    "permission": "multiedit",
                    "pattern": "*",
                    "action": "allow"
                },
                {
                    "permission": "apply_patch",
                    "pattern": "*",
                    "action": "allow"
                },
                {
                    "permission": "external_directory",
                    "pattern": "*",
                    "action": "allow"
                },
                {
                    "permission": "bash",
                    "pattern": "*",
                    "action": "allow"
                },
                {
                    "permission": "doom_loop",
                    "pattern": "*",
                    "action": "allow"
                },
                {
                    "permission": "task",
                    "pattern": "*",
                    "action": "deny"
                }
            ])
        );
    }

    #[test]
    fn workspace_live_sync_permission_rules_preserve_required_mode_for_fenced_native_edits() {
        assert_eq!(
            opencode_workspace_live_sync_permission_rules(
                true,
                crate::provider::AgentPermissionLevel::Required,
            ),
            json!([
                {
                    "permission": "edit",
                    "pattern": "*",
                    "action": "ask"
                },
                {
                    "permission": "write",
                    "pattern": "*",
                    "action": "ask"
                },
                {
                    "permission": "multiedit",
                    "pattern": "*",
                    "action": "ask"
                },
                {
                    "permission": "apply_patch",
                    "pattern": "*",
                    "action": "ask"
                },
                {
                    "permission": "external_directory",
                    "pattern": "*",
                    "action": "ask"
                },
                {
                    "permission": "bash",
                    "pattern": "*",
                    "action": "ask"
                },
                {
                    "permission": "doom_loop",
                    "pattern": "*",
                    "action": "ask"
                },
                {
                    "permission": "task",
                    "pattern": "*",
                    "action": "deny"
                }
            ])
        );
    }

    #[test]
    fn doom_loop_permission_tracks_agent_authority_in_all_rule_sets() {
        for (permission_level, expected_action) in [
            (crate::provider::AgentPermissionLevel::Yolo, "allow"),
            (crate::provider::AgentPermissionLevel::Required, "ask"),
        ] {
            for rules in [
                opencode_permission_rules(permission_level),
                opencode_workspace_live_sync_permission_rules(false, permission_level),
            ] {
                let doom_loop = rules
                    .as_array()
                    .and_then(|rules| {
                        rules.iter().find(|rule| {
                            rule.get("permission").and_then(serde_json::Value::as_str)
                                == Some("doom_loop")
                        })
                    })
                    .expect("OpenCode rules should include doom_loop authority");
                assert_eq!(
                    doom_loop.get("action").and_then(serde_json::Value::as_str),
                    Some(expected_action)
                );
            }
        }
    }

    #[test]
    fn tracked_workspace_live_sync_keeps_opencode_native_tools_enabled() {
        let run = test_run(
            LaunchProviderRequest::new("session-1", "opencode", "opencode", "default", "default")
                .with_workspace_live_sync_mode(crate::config::WorkspaceLiveSyncMode::Tracked),
            false,
        );

        assert!(opencode_workspace_live_sync_native_writes_allowed(&run));
        assert!(!opencode_prompt_should_disable_native_writes(&run));
    }

    #[test]
    fn fenced_managed_workspace_live_sync_keeps_opencode_native_tools_enabled() {
        let run = test_run(
            LaunchProviderRequest::new("session-1", "opencode", "opencode", "default", "default")
                .with_workspace_live_sync_managed(),
            true,
        );

        assert!(opencode_workspace_live_sync_native_writes_allowed(&run));
        assert!(!opencode_prompt_should_disable_native_writes(&run));
    }

    #[test]
    fn unfenced_managed_workspace_live_sync_keeps_opencode_native_tools_kernel_gated() {
        let run = test_run(
            LaunchProviderRequest::new("session-1", "opencode", "opencode", "default", "default")
                .with_workspace_live_sync_managed(),
            false,
        );

        assert!(!opencode_workspace_live_sync_native_writes_allowed(&run));
        assert!(!opencode_prompt_should_disable_native_writes(&run));
        assert!(opencode_prompt_should_allow_native_bash(&run));
    }

    #[test]
    fn managed_workspace_live_sync_prompt_does_not_globally_disable_native_write_tools() {
        let (base_url, received) = prompt_request_server();
        let run = test_run_with_endpoint(
            LaunchProviderRequest::new("session-1", "opencode", "opencode", "default", "default")
                .with_workspace_live_sync_managed(),
            false,
            &base_url,
        );
        let (_tx, rx) = mpsc::channel();
        let mut state = OpenCodeRuntimeState::new(
            base_url,
            "opencode-session-1".to_string(),
            OpenCodeEventSubscription::for_tests(rx),
        );
        let envelope = crate::prompt_assembly::PromptEnvelope::new(
            "edit a sibling repository",
            "",
            Vec::new(),
            crate::prompt_assembly::PromptManifest::default(),
        );

        submit_opencode_prompt(&run, &mut state, &envelope).expect("prompt should submit");

        let body = received.join().expect("server should capture request body");
        assert_eq!(body.pointer("/tools"), None);
        state.stop();
    }

    #[test]
    fn generated_message_ids_use_opencode_sortable_timestamp_width() {
        let id = next_opencode_message_id();

        assert!(id.starts_with("msg_"));
        assert_eq!(id.len(), 30);
        assert!(id[4..16].chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    #[test]
    fn generated_message_ids_are_lexically_monotonic() {
        let first = next_opencode_message_id();
        let second = next_opencode_message_id();

        assert!(
            second > first,
            "generated OpenCode message IDs must sort in creation order: {first} then {second}"
        );
    }

    #[test]
    fn sync_selection_does_not_let_defaults_override_explicit_launch_selection() {
        let selection = resolve_sync_selection(
            &[],
            OpenCodeConfiguredDefaults {
                model: Some("opencode/gpt-5.4".to_string()),
                variant: Some("medium".to_string()),
                ..OpenCodeConfiguredDefaults::default()
            },
            "opencode/gpt-5.4",
            Some("high"),
        );

        assert_eq!(selection.model, None);
        assert_eq!(selection.variant, None);
    }

    #[test]
    fn sync_selection_still_uses_message_metadata_for_explicit_runs() {
        let selection = resolve_sync_selection(
            &[message("opencode", "gpt-5.4", "low")],
            OpenCodeConfiguredDefaults {
                model: Some("opencode/gpt-5.4".to_string()),
                variant: Some("medium".to_string()),
                ..OpenCodeConfiguredDefaults::default()
            },
            "opencode/gpt-5.4",
            Some("high"),
        );

        assert_eq!(selection.model.as_deref(), Some("opencode/gpt-5.4"));
        assert_eq!(selection.variant.as_deref(), Some("low"));
    }

    #[test]
    fn sync_selection_uses_defaults_only_for_unspecified_launch_fields() {
        let selection = resolve_sync_selection(
            &[],
            OpenCodeConfiguredDefaults {
                model: Some("opencode/gpt-5.4".to_string()),
                variant: Some("medium".to_string()),
                ..OpenCodeConfiguredDefaults::default()
            },
            "default",
            None,
        );

        assert_eq!(selection.model.as_deref(), Some("opencode/gpt-5.4"));
        assert_eq!(selection.variant.as_deref(), Some("medium"));
    }

    fn prompt_request_server() -> (String, thread::JoinHandle<serde_json::Value>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("test listener should bind");
        let port = listener
            .local_addr()
            .expect("test listener should expose a local address")
            .port();
        let handle = thread::spawn(move || {
            for request_index in 0..2 {
                let (mut stream, _) = listener.accept().expect("client should connect");
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .expect("read timeout should be set");
                let mut request = Vec::new();
                let mut buf = [0_u8; 1024];
                loop {
                    let size = stream.read(&mut buf).expect("request should read");
                    request.extend_from_slice(&buf[..size]);
                    let request_text = String::from_utf8_lossy(&request);
                    let Some((headers, body)) = request_text.split_once("\r\n\r\n") else {
                        continue;
                    };
                    let content_length = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("Content-Length: "))
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or_default();
                    if body.len() >= content_length {
                        break;
                    }
                }
                let request_text = String::from_utf8_lossy(&request).into_owned();
                if request_index == 0 {
                    let response =
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]";
                    stream
                        .write_all(response.as_bytes())
                        .expect("server should write message baseline");
                    continue;
                }
                let (_, body) = request_text
                    .split_once("\r\n\r\n")
                    .expect("request should include body");
                let body = serde_json::from_str(body).expect("request body should be JSON");
                let response =
                    "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                stream
                    .write_all(response.as_bytes())
                    .expect("server should write response");
                return body;
            }
            unreachable!("prompt server must receive a prompt request")
        });
        (format!("http://127.0.0.1:{port}"), handle)
    }

    fn test_run(request: LaunchProviderRequest, fenced: bool) -> RuntimeProviderRun {
        test_run_with_endpoint(request, fenced, "http://127.0.0.1:1")
    }

    fn test_run_with_endpoint(
        request: LaunchProviderRequest,
        fenced: bool,
        endpoint: &str,
    ) -> RuntimeProviderRun {
        let mut pty_env = BTreeMap::new();
        if fenced {
            pty_env.insert(
                "CHARIOX_WORKSPACE_WRITE_FENCE".to_string(),
                "macos-seatbelt".to_string(),
            );
        }
        RuntimeProviderRun::new(
            "provider-run-1",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "opencode:test".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env,
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some(endpoint.to_string()),
            },
        )
    }
}

pub(super) fn submit_opencode_prompt(
    run: &RuntimeProviderRun,
    state: &mut OpenCodeRuntimeState,
    envelope: &crate::prompt_assembly::PromptEnvelope,
) -> Result<(), DaemonError> {
    let client = OpenCodeClient::new(run.id(), state.base_url())?;
    if let Ok(messages) = client.messages(state.session_id()) {
        state.baseline_existing_messages(&messages);
    }
    let message_id = next_opencode_message_id();
    client.submit_prompt(
        state.session_id(),
        &message_id,
        &envelope.visible_user_prompt,
        &envelope.attachments,
        Some(&envelope.hidden_system_context),
        Some(run.model()),
        run.variant(),
        run.execution_mode(),
        opencode_prompt_should_disable_native_writes(run),
        opencode_prompt_should_allow_native_bash(run),
    )?;
    state.note_prompt_submitted(message_id);
    Ok(())
}

pub(crate) fn run_opencode_utility_prompt(
    run: &RuntimeProviderRun,
    prompt: &str,
    hidden_system_context: &str,
    timeout: Duration,
) -> Result<String, DaemonError> {
    let base_url = run
        .structured_endpoint()
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "opencode_utility_endpoint_missing",
            message: "opencode utility requires a structured provider endpoint".to_string(),
        })?
        .to_string();
    let client = OpenCodeClient::new(run.id(), &base_url)?;
    client.wait_until_healthy(Duration::from_secs(30))?;
    let allow_native_writes = opencode_workspace_live_sync_native_writes_allowed(run);
    let session_permission = if run.requires_workspace_live_sync() {
        Some(opencode_workspace_live_sync_permission_rules(
            allow_native_writes,
            run.permission_level(),
        ))
    } else {
        Some(opencode_permission_rules(run.permission_level()))
    };
    let session_id = client.create_session_with_retry(
        session_permission,
        OPENCODE_SESSION_CREATE_TIMEOUT,
        OPENCODE_SESSION_CREATE_RETRY_INTERVAL,
    )?;
    let event_subscription = client.subscribe_events_with_retry(
        OPENCODE_EVENT_SUBSCRIBE_TIMEOUT,
        OPENCODE_EVENT_SUBSCRIBE_RETRY_INTERVAL,
    )?;
    let mut state = OpenCodeRuntimeState::new(base_url, session_id, event_subscription);
    let envelope = crate::prompt_assembly::PromptEnvelope::new(
        prompt,
        hidden_system_context,
        Vec::new(),
        crate::prompt_assembly::PromptManifest::default(),
    );
    if let Err(error) = submit_opencode_prompt(run, &mut state, &envelope) {
        state.stop();
        return Err(error);
    }

    let deadline = Instant::now() + timeout;
    let mut output = String::new();
    let mut completed = false;
    while Instant::now() < deadline {
        let drain = match drain_opencode_events(run, &mut state, None) {
            Ok(drain) => drain,
            Err(error) => {
                state.stop();
                return Err(error);
            }
        };
        for chunk in drain.chunks {
            if chunk.kind == TerminalOutputKind::ProviderOutput {
                output.push_str(&String::from_utf8_lossy(&chunk.bytes));
            }
        }
        if let Some(failure) = drain.terminal_failure {
            state.stop();
            return Err(DaemonError::ProviderProtocol {
                provider_run_id: run.id().to_string(),
                operation: "opencode_utility_failed",
                message: failure,
            });
        }
        if drain.prompt_completed {
            completed = true;
            break;
        }
        std::thread::sleep(OPENCODE_UTILITY_POLL_INTERVAL);
    }
    if !completed {
        let _ = OpenCodeClient::new(run.id(), state.base_url())
            .and_then(|client| client.abort_session(state.session_id()));
        state.stop();
        return Err(DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "opencode_utility_timeout",
            message: format!(
                "opencode utility did not complete within {} ms",
                timeout.as_millis()
            ),
        });
    }
    state.stop();
    let output = clean_opencode_utility_output(&output);
    if output.is_empty() {
        return Err(DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "opencode_utility_empty_output",
            message: "opencode utility returned no assistant text".to_string(),
        });
    }
    Ok(output)
}

fn clean_opencode_utility_output(output: &str) -> String {
    let trimmed = output.trim();
    if let Some(stripped) = trimmed
        .strip_prefix("```json")
        .and_then(|value| value.strip_suffix("```"))
    {
        return stripped.trim().to_string();
    }
    if let Some(stripped) = trimmed
        .strip_prefix("```")
        .and_then(|value| value.strip_suffix("```"))
    {
        return stripped.trim().to_string();
    }
    trimmed.to_string()
}

fn next_opencode_message_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default();
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed) & 0x0fff;
    let encoded_time =
        timestamp_ms.saturating_mul(0x1000).saturating_add(sequence) & 0xffff_ffff_ffff;
    let random = Alphanumeric.sample_string(&mut rand::thread_rng(), 14);
    format!("msg_{encoded_time:012x}{random}")
}

fn resolve_initial_selection(
    run: &RuntimeProviderRun,
    client: &OpenCodeClient,
) -> Result<OpenCodeRunSelection, DaemonError> {
    if run.model() != "default" && run.variant().is_some() {
        crate::logging::debug_with_fields(
            "daemon.provider.opencode",
            "skipped configured defaults lookup for explicit model and variant",
            serde_json::json!({
                "provider_run_id": run.id(),
                "requested_model": run.model(),
                "requested_variant": run.variant(),
            }),
        );
        return Ok(OpenCodeRunSelection::default());
    }

    let resolved = client.configured_defaults()?;
    crate::logging::debug_with_fields(
        "daemon.provider.opencode",
        "checked opencode configured defaults",
        serde_json::json!({
            "provider_run_id": run.id(),
            "requested_model": run.model(),
            "requested_variant": run.variant(),
            "selected_agent": resolved.selected_agent,
            "agent_model": resolved.agent_model,
            "agent_variant": resolved.agent_variant,
            "top_level_model": resolved.top_level_model,
            "resolved_model": resolved.model,
            "resolved_variant": resolved.variant,
        }),
    );

    Ok(OpenCodeRunSelection {
        model: (run.model() == "default")
            .then_some(resolved.model)
            .flatten(),
        variant: run
            .variant()
            .is_none()
            .then_some(resolved.variant)
            .flatten(),
    })
}
