//! MP-08 / MP-10 / MP-11: projection cost, encoded boundaries and bounded retention.
use super::*;
use crate::history::{HistoryEventTurnContext, OperationalHistoryStore};

#[test]
fn public_history_stream_reprojects_only_secret_boundary() {
    // MP-08 / MP-10 / MP-11: old text is sanitized; new split values still scrub.
    use std::sync::atomic::{AtomicUsize, Ordering};
    let root = std::env::temp_dir().join(format!(
        "chariox-am9-boundary-{:016x}",
        rand::random::<u64>()
    ));
    std::fs::create_dir(&root).unwrap();
    let protection = room_secret_observation::RoomSecretObservations::new(
        root.join("observations"),
        Default::default(),
    );
    let store = OperationalHistoryStore::open(root.join("history.sqlite")).unwrap();
    let max_bytes = Arc::new(AtomicUsize::new(0));
    let measured = max_bytes.clone();
    let projector = protection.clone();
    store.set_public_history_projector(Arc::new(move |event| {
        measured.fetch_max(
            event.content.as_ref().map_or(0, String::len),
            Ordering::Relaxed,
        );
        projector.public_history_document(event, "owner")
    }));
    let protection = protection.with_public_history(store.clone());
    let secret = "cross-boundary-secret";
    {
        let guard = store.lock_public_history().unwrap();
        protection
            .register_stored_with_locked_public_history("room", secret)
            .unwrap();
        drop(guard);
    }
    let append = |text: String| {
        store
            .append_transcript(
                &crate::history::SessionHistoryEntry::provider_output(
                    "room",
                    "run",
                    Some("peer"),
                    crate::terminal::TerminalOutputKind::ProviderOutput,
                    Some("message".into()),
                    text,
                ),
                crate::history::HistoryEventTurnContext {
                    public_history_owner_user_id: Some("owner".into()),
                    session_id: Some("room".into()),
                    agent_id: Some("peer".into()),
                    provider_run_id: Some("run".into()),
                    prompt_id: Some("prompt".into()),
                    ..Default::default()
                },
            )
            .unwrap()
    };
    let first = append("界".repeat(10_000));
    max_bytes.store(0, Ordering::Relaxed);
    append("next".into());
    assert!(
        max_bytes.load(Ordering::Relaxed) <= secret.len() * 6 + 4,
        "must not project the assembled answer"
    );
    append(" cross-boundary-".into());
    append("secret".into());
    let guard = store.lock_public_history().unwrap();
    let doc = store
        .read_public_history_locked("owner", "room", &first.event_id)
        .unwrap()
        .unwrap();
    assert!(doc.text.ends_with("next [redacted]"));
    assert!(!doc.text.contains(secret));
    drop(guard);
    drop(protection);
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn public_history_boundary_scrubs_encoded_values_after_registry_growth() {
    // MP-08 / MP-10 / MP-11: cache refresh and JSON/encoded split values.
    let root = std::env::temp_dir().join(format!(
        "chariox-am9-encoded-{:016x}",
        rand::random::<u64>()
    ));
    std::fs::create_dir(&root).unwrap();
    let protection = room_secret_observation::RoomSecretObservations::new(
        root.join("observations"),
        Default::default(),
    );
    let store = OperationalHistoryStore::open(root.join("history.sqlite")).unwrap();
    let projector = protection.clone();
    store.set_public_history_projector(Arc::new(move |event| {
        projector.public_history_document(event, event.public_history_owner_user_id.as_deref()?)
    }));
    let protection = protection.with_public_history(store.clone());
    let append = |text: String| {
        store
            .append_transcript(
                &crate::history::SessionHistoryEntry::provider_output(
                    "room",
                    "run",
                    Some("peer"),
                    crate::terminal::TerminalOutputKind::ProviderOutput,
                    Some("message".into()),
                    text,
                ),
                crate::history::HistoryEventTurnContext {
                    public_history_owner_user_id: Some("owner".into()),
                    session_id: Some("room".into()),
                    agent_id: Some("peer".into()),
                    provider_run_id: Some("run".into()),
                    prompt_id: Some("prompt".into()),
                    ..Default::default()
                },
            )
            .unwrap()
    };
    let first = append("old public prefix ".repeat(1000));
    append("first delta ".into()); // caches an empty registry boundary
    append(String::new()); // an empty delta must not remove the earlier message
    {
        let guard = store.lock_public_history().unwrap();
        assert!(store
            .read_public_history_locked("owner", "room", &first.event_id)
            .unwrap()
            .is_some());
        drop(guard);
    }
    let secret = "quote-\"-longer-new-secret";
    {
        let guard = store.lock_public_history().unwrap();
        protection
            .register_stored_with_locked_public_history("room", secret)
            .unwrap();
        drop(guard);
    }
    use base64::Engine as _;
    for variant in [
        secret.to_string(),
        secret.to_uppercase(),
        secret.bytes().map(|b| format!("{b:02x}")).collect(),
        base64::engine::general_purpose::STANDARD.encode(secret),
        serde_json::to_string(secret)
            .unwrap()
            .trim_matches('"')
            .to_owned(),
    ] {
        append(" public ".into());
        let split = variant.len() / 2;
        append(variant[..split].to_owned());
        append(variant[split..].to_owned());
        let guard = store.lock_public_history().unwrap();
        let doc = store
            .read_public_history_locked("owner", "room", &first.event_id)
            .unwrap()
            .unwrap();
        assert!(
            doc.text.ends_with("public [redacted]"),
            "encoded boundary was not scrubbed"
        );
        drop(guard);
    }
    // Missing trusted provenance cannot remove or mutate an existing stream.
    let mut untrusted = first.clone();
    untrusted.event_id = "untrusted".into();
    untrusted.sequence = store.reserve_sequence();
    untrusted.public_history_owner_user_id = None;
    untrusted.content = Some("forged tail".into());
    store.append(&untrusted).unwrap();
    let guard = store.lock_public_history().unwrap();
    assert!(!store
        .read_public_history_locked("owner", "room", &first.event_id)
        .unwrap()
        .unwrap()
        .text
        .contains("forged tail"));
    drop(guard);
    drop(protection);
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn public_history_registration_removes_multiple_candidate_chunks() {
    // MP-08 / MP-10 / MP-11: candidates beyond a chunk boundary are never retained.
    let root =
        std::env::temp_dir().join(format!("chariox-am9-chunks-{:016x}", rand::random::<u64>()));
    std::fs::create_dir(&root).unwrap();
    let protection = room_secret_observation::RoomSecretObservations::new(
        root.join("observations"),
        Default::default(),
    );
    let store = OperationalHistoryStore::open(root.join("history.sqlite")).unwrap();
    let projector = protection.clone();
    store.set_public_history_projector(Arc::new(move |event| {
        projector.public_history_document(event, "owner")
    }));
    let protection = protection.with_public_history(store.clone());
    let first = store
        .append_operational_event(
            HistoryEventKind::UserPrompt,
            None,
            Some("useful".into()),
            Default::default(),
            crate::history::HistoryEventTurnContext {
                session_id: Some("room".into()),
                agent_id: Some("peer".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let secret = "quoted-\"-value";
    let events: Vec<_> = (0..300)
        .map(|i| {
            let mut e = first.clone();
            e.event_id = format!("chunk-{i}");
            e.sequence = store.reserve_sequence();
            e.content = Some(format!("public {secret}"));
            e
        })
        .collect();
    store.append_many(&events).unwrap();
    protection.register("room", secret).unwrap();
    assert_eq!(
        protection
            .history_retention_checks
            .load(std::sync::atomic::Ordering::Relaxed),
        300
    );
    let guard = store.lock_public_history().unwrap();
    for e in &events {
        assert!(store
            .read_public_history_locked("owner", "room", &e.event_id)
            .unwrap()
            .is_none());
    }
    assert!(store
        .read_public_history_locked("owner", "room", &first.event_id)
        .unwrap()
        .is_some());
    drop(guard);
    drop(protection);
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn public_history_registration_checks_only_new_value_candidates() {
    // MP-08 / MP-10 / MP-11: credential registration should not project unrelated Room history.
    let root =
        std::env::temp_dir().join(format!("chariox-am9-retain-{:016x}", rand::random::<u64>()));
    std::fs::create_dir(&root).unwrap();
    let protection = room_secret_observation::RoomSecretObservations::new(
        root.join("observations"),
        Default::default(),
    );
    let store = OperationalHistoryStore::open(root.join("history.sqlite")).unwrap();
    let projector = protection.clone();
    store.set_public_history_projector(Arc::new(move |event| {
        projector.public_history_document(event, "owner")
    }));
    let protection = protection.with_public_history(store.clone());
    let first = store
        .append_operational_event(
            HistoryEventKind::UserPrompt,
            None,
            Some("unrelated useful review".into()),
            Default::default(),
            HistoryEventTurnContext {
                session_id: Some("room".into()),
                agent_id: Some("peer".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let events: Vec<_> = (0..600)
        .map(|i| {
            let mut e = first.clone();
            e.event_id = format!("retain-{i}");
            e.sequence = store.reserve_sequence();
            if i == 300 {
                e.content = Some("matching fresh-canary".into());
            }
            e
        })
        .collect();
    store.append_many(&events).unwrap();
    protection.register("room", "fresh-canary").unwrap();
    assert_eq!(
        protection
            .history_retention_checks
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "only serialized candidates need the full secret scrub"
    );
    let guard = store.lock_public_history().unwrap();
    assert!(store
        .read_public_history_locked("owner", "room", &events[300].event_id)
        .unwrap()
        .is_none());
    assert!(store
        .read_public_history_locked("owner", "room", &events[0].event_id)
        .unwrap()
        .is_some());
    drop(guard);
    drop(protection);
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn public_history_batch_withholds_secret_reference_collision() {
    // MP-08 / MP-10 / MP-11: every batch alias retains the canonical identity fence.
    let root =
        std::env::temp_dir().join(format!("chariox-am9-refs-{:016x}", rand::random::<u64>()));
    std::fs::create_dir(&root).unwrap();
    let protection = room_secret_observation::RoomSecretObservations::new(
        root.join("observations"),
        Default::default(),
    );
    let store = OperationalHistoryStore::open(root.join("history.sqlite")).unwrap();
    let projector = protection.clone();
    store.set_public_history_projector(Arc::new(move |event| {
        projector.public_history_document(event, "owner")
    }));
    let protection = protection.with_public_history(store.clone());
    {
        let guard = store.lock_public_history().unwrap();
        protection
            .register_stored_with_locked_public_history("room", "batch-sensitive-ref")
            .unwrap();
        drop(guard);
    }
    let first = crate::history::HistoryEvent::transcript(
        store.reserve_sequence(),
        &crate::history::SessionHistoryEntry::provider_output(
            "room",
            "run",
            Some("peer"),
            crate::terminal::TerminalOutputKind::ProviderOutput,
            Some("message".into()),
            "public answer".to_owned(),
        ),
        HistoryEventTurnContext {
            public_history_owner_user_id: Some("owner".into()),
            session_id: Some("room".into()),
            agent_id: Some("peer".into()),
            provider_run_id: Some("run".into()),
            prompt_id: Some("prompt".into()),
            ..Default::default()
        },
    );
    let mut next = first.clone();
    next.sequence = store.reserve_sequence();
    next.event_id = "batch-sensitive-ref".into();
    store.append_many(&[first.clone(), next]).unwrap();
    let guard = store.lock_public_history().unwrap();
    assert!(store
        .read_public_history_locked("owner", "room", &first.event_id)
        .unwrap()
        .is_none());
    drop(guard);
    drop(protection);
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn public_history_worker_provider_provenance_survives_local_run_collision() {
    // MP-08 / MP-10 / MP-11: exercise authenticated projection and real fan-out.
    use crate::provider::{
        AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
    };
    use crate::transport::relay_peer::{RelayPeerEvent, RelayProjectedOutputChunk};
    let root = std::env::temp_dir().join(format!(
        "chariox-am9-worker-provenance-{:016x}",
        rand::random::<u64>()
    ));
    std::fs::create_dir(&root).unwrap();
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.history.operational.path =
        Some(root.join("history.sqlite").display().to_string());
    let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new("home", "home"))
        .unwrap();
    let make_run = |provider: &str, session_id: &str, agent_id: &str| {
        RuntimeProviderRun::new(
            "provider-run-1",
            &LaunchProviderRequest::new(session_id, provider, provider, "default", "default")
                .with_agent_id(agent_id),
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "provenance-test".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: None,
            },
        )
    };
    app.providers_mut()
        .insert_run_for_test(make_run("opencode", session.id(), "local-agent"));
    app.agents
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel-1".into(),
                worker_machine_id: "worker-machine".into(),
                execution_lease_id: "execution-lease".into(),
                leased_agent_id: "leased-agent".into(),
                active_worker_provider_run_id: Some("provider-run-1".into()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    let protection = room_secret_observation::RoomSecretObservations::new(
        root.join("observations"),
        Default::default(),
    );
    let store = app.operational_history_store();
    store.set_public_history_projector(Arc::new(move |event| {
        protection.public_history_document(event, event.public_history_owner_user_id.as_deref()?)
    }));
    let outcome = crate::app::RemoteLeaseRuntime::new(&mut app).project_remote_runtime_projection(
        crate::runtime::relay_peer_authority::test_projection_authority("worker-kernel-1"),
        RelayPeerEvent::LeasedRuntimeProjection {
            home_session_id: session.id().into(), home_agent_id: agent.id().into(), provider_run_id: "provider-run-1".into(),
            provider_run: Some(make_run("codex", "worker-session", "worker-agent")), prompts: vec![],
            output_chunks: vec![
                RelayProjectedOutputChunk { kind: crate::terminal::TerminalOutputKind::ProviderTool, merge_key: Some("mcp".into()), bytes: br#"{"tool":"read","title":"files","output":"unregistered_private_canary"}"#.to_vec() },
                RelayProjectedOutputChunk { kind: crate::terminal::TerminalOutputKind::ProviderTool, merge_key: Some("native".into()), bytes: br#"{"tool":"bash","description":"cwd /worker","input":{"command":"printf worker_native_canary","cwd":"/worker"},"output":"worker_native_canary"}"#.to_vec() },
            ], notices: vec![], completions: vec![],
        }).unwrap();
    assert!(outcome.accepted);
    let guard = store.lock_public_history().unwrap();
    let private = store
        .search_public_history_locked(
            session.owner_user_id(),
            session.id(),
            None,
            "unregistered_private_canary",
            50,
            None,
        )
        .unwrap();
    assert!(
        private.hits.is_empty(),
        "worker MCP was classified using the colliding local OpenCode run"
    );
    let private_events = store
        .query_events(crate::history::HistoryEventQuery {
            session_id: Some(session.id().to_owned()),
            text: Some("unregistered_private_canary".into()),
            limit: Some(10),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        private_events.len(),
        1,
        "the fixture must reach operational history"
    );
    assert_eq!(private_events[0].provider.as_deref(), Some("codex"));
    assert!(
        store
            .read_public_history_locked(
                session.owner_user_id(),
                session.id(),
                &private_events[0].event_id,
            )
            .unwrap()
            .is_none(),
        "the entire worker MCP record must be unavailable by reference"
    );
    let native = store
        .search_public_history_locked(
            session.owner_user_id(),
            session.id(),
            None,
            "worker_native_canary",
            50,
            None,
        )
        .unwrap();
    assert_eq!(
        native.hits.len(),
        1,
        "admitted worker-native tools must remain public"
    );
    let detail = store
        .read_public_history_locked(
            session.owner_user_id(),
            session.id(),
            &native.hits[0].event_ref,
        )
        .unwrap()
        .unwrap();
    assert!(detail.text.contains("worker_native_canary"));
    assert!(!detail.text.contains("unregistered_private_canary"));
    drop(guard);
    drop(store);
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}
