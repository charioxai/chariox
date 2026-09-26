use chariox_app_package::{pack, verify, Limits, Manifest, TrustedPublisher, VerificationPolicy};
use chariox_app_runtime::app_inbox::{
    self, Accepted, InboxError, InboxRoute, InboxState, IncomingCatalog, MAX_ATTEMPTS,
    PENDING_LIFETIME_MS,
};
use ed25519_dalek::SigningKey;
use rusqlite::Connection;
use serde_json::json;
use std::collections::BTreeMap;

fn catalog() -> IncomingCatalog {
    let key = SigningKey::from_bytes(&[41; 32]);
    let manifest: Manifest = serde_json::from_value(json!({
        "schema":"chariox.app.v1","appId":"com.example.inbox","version":"1.0.0",
        "publisher":{"id":"com.example","keyId":"developer","name":"Developer"},
        "sdkVersion":"0.8.0","appContractVersion":1,"minKernelProtocol":500,
        "resourcePolicy":"chariox.app.resources.v1","runtime":{"engine":"node","entry":"runtime/main.js"},
        "ui":{"entry":"ui/index.html"},"events":"schemas/events.json","capabilities":{}
    }))
    .unwrap();
    let text = json!({"type":"object","additionalProperties":false,"required":["text"],
        "properties":{"text":{"type":"string"}}});
    let files = BTreeMap::from([
        (
            "runtime/main.js".into(),
            b"export default function register() {}".to_vec(),
        ),
        (
            "ui/index.html".into(),
            b"<!doctype html><title>Inbox</title>".to_vec(),
        ),
        (
            "schemas/events.json".into(),
            serde_json::to_vec(&json!({"events":[
                {"name":"received","schemaVersion":2,"direction":"incoming","payloadSchema":text},
                {"name":"sync","schemaVersion":1,"direction":"both","payloadSchema":text},
                {"name":"changed","schemaVersion":1,"direction":"outgoing","payloadSchema":text},
            ]}))
            .unwrap(),
        ),
    ]);
    let bytes = pack(&manifest, &files, &key, &Limits::default()).unwrap();
    let publisher = TrustedPublisher {
        publisher_id: "com.example".into(),
        key_id: "developer".into(),
        public_key: key.verifying_key(),
    };
    let verified = verify(&bytes, &VerificationPolicy::new(500, vec![publisher])).unwrap();
    IncomingCatalog::compile(&verified).unwrap()
}

fn db() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    app_inbox::initialize(&db).unwrap();
    db
}

fn route(id: &str) -> InboxRoute {
    InboxRoute {
        route_id: id.into(),
        owner_id: "owner".into(),
        installation_id: "installed".into(),
        event_name: "received".into(),
        source_event_type: "dev.chariox.dummy/dummy.test".into(),
        source_event_version: 1,
        active: true,
    }
}

#[test]
fn catalog_holds_incoming_and_both_events_only() {
    let catalog = catalog();
    assert_eq!(catalog.version("received"), Some(2));
    assert_eq!(catalog.version("sync"), Some(1));
    assert_eq!(catalog.version("changed"), None);
    catalog.validate("received", &json!({"text":"hi"})).unwrap();
    assert!(matches!(
        catalog.validate("received", &json!({"text":1})),
        Err(InboxError::Schema)
    ));
    assert!(matches!(
        catalog.validate("changed", &json!({"text":"hi"})),
        Err(InboxError::Schema)
    ));
}

#[test]
fn routes_are_owned_and_unique() {
    let db = db();
    app_inbox::create_route_in(&db, &route("r1"), 1).unwrap();
    assert!(matches!(
        app_inbox::create_route_in(&db, &route("r1"), 1),
        Err(InboxError::Conflict)
    ));
    assert_eq!(
        app_inbox::routes(&db, "owner", "installed").unwrap(),
        vec![route("r1")]
    );
    assert!(matches!(
        app_inbox::remove_route_in(&db, "someone", "installed", "r1"),
        Err(InboxError::NotFound)
    ));
    app_inbox::remove_route_in(&db, "owner", "installed", "r1").unwrap();
    assert!(app_inbox::route(&db, "owner", "installed", "r1")
        .unwrap()
        .is_none());
}

#[test]
fn accepting_dedupes_by_source_occurrence() {
    let db = db();
    let route = route("r1");
    let Accepted::New(first) =
        app_inbox::accept_in(&db, &route, "occ-1", &json!({"text":"a"}), 3, 100).unwrap()
    else {
        panic!("first acceptance must be new");
    };
    assert_eq!(
        app_inbox::accept_in(&db, &route, "occ-1", &json!({"text":"a"}), 3, 200).unwrap(),
        Accepted::Duplicate(first)
    );
    assert!(matches!(
        app_inbox::accept_in(&db, &route, "occ-1", &json!({"text":"b"}), 3, 200),
        Err(InboxError::Conflict)
    ));
    let inactive = InboxRoute {
        active: false,
        ..route.clone()
    };
    assert!(matches!(
        app_inbox::accept_in(&db, &inactive, "occ-2", &json!({}), 3, 200),
        Err(InboxError::NotFound)
    ));
    let due = app_inbox::due(&db, 100, 10).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].occurrence_id, "occ-1");
    assert_eq!(due[0].payload, json!({"text":"a"}));
    app_inbox::delivered_in(&db, first, 3).unwrap();
    assert_eq!(
        app_inbox::counts(&db, &route).unwrap(),
        app_inbox::InboxCounts {
            delivered: 1,
            ..Default::default()
        }
    );
    assert_eq!(app_inbox::state(&db, first).unwrap(), InboxState::Delivered);
    assert!(app_inbox::due(&db, 100, 10).unwrap().is_empty());
    // A settled occurrence stays deduplicated; a replay does not redeliver.
    assert_eq!(
        app_inbox::accept_in(&db, &route, "occ-1", &json!({"text":"a"}), 3, 300).unwrap(),
        Accepted::Duplicate(first)
    );
}

#[test]
fn settled_occurrences_are_pruned_after_the_dedupe_window() {
    let db = db();
    let route = route("r1");
    let Accepted::New(first) =
        app_inbox::accept_in(&db, &route, "occ-old", &json!({"text":"a"}), 3, 1_000).unwrap()
    else {
        panic!("first acceptance must be new");
    };
    app_inbox::delivered_in(&db, first, 3).unwrap();
    // Inside the window a replay is still a duplicate.
    let inside = 1_000 + app_inbox::DEDUPE_WINDOW_MS - 1;
    assert_eq!(
        app_inbox::accept_in(&db, &route, "occ-old", &json!({"text":"a"}), 3, inside).unwrap(),
        Accepted::Duplicate(first)
    );
    // Past it, the next acceptance prunes the settled row.
    let past = 1_000 + app_inbox::DEDUPE_WINDOW_MS;
    app_inbox::accept_in(&db, &route, "occ-new", &json!({"text":"b"}), 3, past).unwrap();
    assert_eq!(
        app_inbox::counts(&db, &route).unwrap(),
        app_inbox::InboxCounts {
            pending: 1,
            ..Default::default()
        }
    );
}

#[test]
fn failures_back_off_then_poison() {
    let db = db();
    let Accepted::New(sequence) =
        app_inbox::accept_in(&db, &route("r1"), "occ", &json!({"text":"a"}), 1, 0).unwrap()
    else {
        panic!();
    };
    let mut now = 0;
    for attempt in 1..MAX_ATTEMPTS {
        assert_eq!(
            app_inbox::failed_attempt_in(&db, sequence, now).unwrap(),
            InboxState::Retryable
        );
        assert!(
            app_inbox::due(&db, now, 10).unwrap().is_empty(),
            "attempt {attempt} backs off"
        );
        now += 60_000;
        let due = app_inbox::due(&db, now, 10).unwrap();
        assert_eq!(due[0].attempts, attempt);
    }
    assert_eq!(
        app_inbox::failed_attempt_in(&db, sequence, now).unwrap(),
        InboxState::Failed
    );
    assert!(app_inbox::due(&db, u64::MAX / 2, 10).unwrap().is_empty());
    assert!(matches!(
        app_inbox::delivered_in(&db, sequence, 1),
        Err(InboxError::NotFound)
    ));
}

#[test]
fn postponing_spends_no_attempt_and_old_occurrences_expire() {
    let db = db();
    let Accepted::New(sequence) =
        app_inbox::accept_in(&db, &route("r1"), "occ", &json!({"text":"a"}), 1, 0).unwrap()
    else {
        panic!();
    };
    app_inbox::postpone_in(&db, sequence, 5_000).unwrap();
    assert!(app_inbox::due(&db, 4_999, 10).unwrap().is_empty());
    assert_eq!(app_inbox::due(&db, 5_000, 10).unwrap()[0].attempts, 0);
    assert!(app_inbox::due(&db, PENDING_LIFETIME_MS, 10)
        .unwrap()
        .is_empty());
    assert_eq!(
        app_inbox::state(&db, sequence).unwrap(),
        InboxState::Expired
    );
}

#[test]
fn routes_are_scoped_per_owner_and_installation_and_removal_forgets_occurrences() {
    let db = db();
    app_inbox::create_route_in(&db, &route("mail"), 1).unwrap();
    let other = InboxRoute {
        owner_id: "someone".into(),
        ..route("mail")
    };
    // Another owner's route of the same name neither conflicts nor sees it.
    app_inbox::create_route_in(&db, &other, 1).unwrap();
    app_inbox::accept_in(&db, &route("mail"), "occ-1", &json!({"text":"a"}), 1, 0).unwrap();
    assert!(matches!(
        app_inbox::accept_in(&db, &other, "occ-1", &json!({"text":"b"}), 1, 0).unwrap(),
        Accepted::New(_)
    ));
    assert_eq!(app_inbox::counts(&db, &other).unwrap().pending, 1);
    app_inbox::remove_route_in(&db, "owner", "installed", "mail").unwrap();
    app_inbox::create_route_in(&db, &route("mail"), 2).unwrap();
    assert_eq!(
        app_inbox::counts(&db, &route("mail")).unwrap(),
        app_inbox::InboxCounts::default()
    );
    assert!(matches!(
        app_inbox::accept_in(&db, &route("mail"), "occ-1", &json!({"text":"c"}), 1, 3).unwrap(),
        Accepted::New(_)
    ));
    let due = app_inbox::due(&db, 10, 10).unwrap();
    assert_eq!(due.len(), 2);
    assert!(due.iter().all(|item| item.accepted_generation == 1));
    app_inbox::undeliverable_in(&db, due[0].sequence).unwrap();
    assert_eq!(
        app_inbox::state(&db, due[0].sequence).unwrap(),
        InboxState::Failed
    );
}
