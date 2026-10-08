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
