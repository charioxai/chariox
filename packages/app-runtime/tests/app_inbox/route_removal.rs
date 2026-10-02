use super::*;

#[test]
fn removed_route_retains_accepted_delivery_and_dedupe_across_reopen() {
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "chariox-b7-inbox-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    std::fs::create_dir(&scratch.0).unwrap();
    let path = scratch.0.join("inbox.sqlite");
    let db = Connection::open(&path).unwrap();
    app_inbox::initialize(&db).unwrap();
    let original = route("mail");
    app_inbox::create_route_in(&db, &original, 1).unwrap();
    let Accepted::New(sequence) =
        app_inbox::accept_in(&db, &original, "occ-1", &json!({"text":"a"}), 3, 100).unwrap()
    else {
        panic!("first acceptance must be new");
    };
    app_inbox::postpone_in(&db, sequence, 200).unwrap();
    app_inbox::remove_route_in(&db, "owner", "installed", "mail").unwrap();
    assert!(app_inbox::route(&db, "owner", "installed", "mail")
        .unwrap()
        .is_none());
    drop(db);

    let db = Connection::open(&path).unwrap();
    app_inbox::initialize(&db).unwrap();
    assert!(app_inbox::due(&db, 199, 10).unwrap().is_empty());
    let due = app_inbox::due(&db, 200, 10).unwrap();
    assert_eq!(
        due.len(),
        1,
        "an acknowledged occurrence survives route removal"
    );
    assert_eq!(due[0].sequence, sequence);
    assert_eq!(due[0].event_name, "received");
    assert_eq!(due[0].payload, json!({"text":"a"}));
    assert_eq!(due[0].accepted_generation, 3);
    app_inbox::delivered_in(&db, sequence, 4).unwrap();
    assert!(app_inbox::due(&db, 200, 10).unwrap().is_empty());
    assert!(matches!(
        app_inbox::delivered_in(&db, sequence, 4),
        Err(InboxError::NotFound)
    ));

    // Reusing a route name cannot redeliver a settled occurrence or retarget
    // its old payload. A fresh source identity can enter the replacement route.
    let replacement = InboxRoute {
        event_name: "sync".into(),
        ..original.clone()
    };
    app_inbox::create_route_in(&db, &replacement, 201).unwrap();
    assert_eq!(
        app_inbox::accept_in(&db, &replacement, "occ-1", &json!({"text":"a"}), 4, 202).unwrap(),
        Accepted::Duplicate(sequence)
    );
    assert!(matches!(
        app_inbox::accept_in(&db, &replacement, "occ-1", &json!({"text":"b"}), 4, 202),
        Err(InboxError::Conflict)
    ));
    let Accepted::New(next) =
        app_inbox::accept_in(&db, &replacement, "occ-2", &json!({"text":"b"}), 4, 202).unwrap()
    else {
        panic!()
    };
    assert_ne!(next, sequence);
    let due = app_inbox::due(&db, 202, 10).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].event_name, "sync");
    assert_eq!(due[0].accepted_generation, 4);
    app_inbox::delivered_in(&db, next, 4).unwrap();
    app_inbox::remove_route_in(&db, "owner", "installed", "mail").unwrap();
    app_inbox::create_route_in(&db, &original, 203).unwrap();
    assert_eq!(app_inbox::counts(&db, &original).unwrap().delivered, 2);
    assert_eq!(
        app_inbox::accept_in(&db, &original, "occ-1", &json!({"text":"a"}), 4, 204).unwrap(),
        Accepted::Duplicate(sequence)
    );
    assert!(app_inbox::due(&db, 204, 10).unwrap().is_empty());
    // Uninstall remains the explicit boundary which discards installation data.
    app_inbox::remove_all_routes_in(&db, "owner", "installed").unwrap();
    assert_eq!(
        app_inbox::counts(&db, &original).unwrap(),
        app_inbox::InboxCounts::default()
    );
}

#[test]
fn removed_routes_keep_retry_order_and_do_not_touch_another_owner() {
    let db = db();
    let original = route("mail");
    let other = InboxRoute {
        owner_id: "other".into(),
        ..original.clone()
    };
    app_inbox::create_route_in(&db, &original, 1).unwrap();
    app_inbox::create_route_in(&db, &other, 1).unwrap();
    // Arrival order, rather than lexical source occurrence order, is retained.
    let mut sequences = Vec::new();
    for (occurrence, text) in [("z-late", "late"), ("a-early", "early")] {
        let Accepted::New(sequence) =
            app_inbox::accept_in(&db, &original, occurrence, &json!({"text":text}), 3, 100)
                .unwrap()
        else {
            panic!()
        };
        sequences.push(sequence);
    }
    app_inbox::failed_attempt_in(&db, sequences[0], 100).unwrap();
    app_inbox::postpone_in(&db, sequences[1], 60_100).unwrap();
    let Accepted::New(foreign) =
        app_inbox::accept_in(&db, &other, "z-late", &json!({"text":"other"}), 3, 100).unwrap()
    else {
        panic!()
    };
    app_inbox::remove_route_in(&db, "owner", "installed", "mail").unwrap();
    assert_eq!(app_inbox::due(&db, 100, 10).unwrap()[0].sequence, foreign);
    app_inbox::delivered_in(&db, foreign, 3).unwrap();
    let due = app_inbox::due(&db, 60_100, 10).unwrap();
    assert_eq!(
        due.iter().map(|item| item.sequence).collect::<Vec<_>>(),
        sequences
    );
    assert_eq!(due[0].attempts, 1);
    assert_eq!(due[1].attempts, 0);
    for item in due {
        app_inbox::delivered_in(&db, item.sequence, 4).unwrap();
    }
    assert!(app_inbox::due(&db, 60_100, 10).unwrap().is_empty());
    assert_eq!(app_inbox::counts(&db, &other).unwrap().delivered, 1);
    assert!(app_inbox::route(&db, "other", "installed", "mail")
        .unwrap()
        .is_some());
}

#[test]
fn removed_route_receipts_expire_at_the_existing_retention_boundary() {
    let db = db();
    let original = route("mail");
    app_inbox::create_route_in(&db, &original, 1).unwrap();
    let Accepted::New(sequence) =
        app_inbox::accept_in(&db, &original, "occ-1", &json!({"text":"a"}), 3, 100).unwrap()
    else {
        panic!();
    };
    app_inbox::delivered_in(&db, sequence, 3).unwrap();
    app_inbox::remove_route_in(&db, "owner", "installed", "mail").unwrap();
    let boundary = 100 + app_inbox::DEDUPE_WINDOW_MS;
    assert!(app_inbox::due(&db, boundary - 1, 10).unwrap().is_empty());
    assert_eq!(app_inbox::counts(&db, &original).unwrap().delivered, 1);
    assert!(app_inbox::due(&db, boundary, 10).unwrap().is_empty());
    assert_eq!(app_inbox::counts(&db, &original).unwrap().delivered, 0);
}
