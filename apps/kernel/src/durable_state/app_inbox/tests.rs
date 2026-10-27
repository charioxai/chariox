use super::*;
use serde_json::json;

#[test]
fn writer_removal_refuses_new_acceptance_but_keeps_acknowledged_work() {
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "chariox-b7-inbox-writer-{:016x}",
        rand::random::<u64>()
    )));
    std::fs::create_dir(&scratch.0).unwrap();
    let path = scratch.0.join("kernel.sqlite");
    let store = DurableKernelStateStore::open_owned(path.clone()).unwrap();
    crate::durable_state::app_state::fixture_inbox_installation(&store, "alice");
    let route = InboxRoute {
        route_id: "mail".into(),
        owner_id: "alice".into(),
        installation_id: "installed".into(),
        event_name: "received".into(),
        source_event_type: "dev.chariox.dummy/dummy.test".into(),
        source_event_version: 1,
        active: true,
        source: None,
    };
    store
        .app_inbox(AppInboxOperation::CreateRoute {
            route: route.clone(),
            now_ms: 1,
        })
        .unwrap();
    let accept = |id: &str| AppInboxOperation::Accept {
        owner: "alice".into(),
        installation: "installed".into(),
        route_id: "mail".into(),
        occurrence_id: id.into(),
        payload: json!({"text":"kept"}),
        generation: 1,
        now_ms: 100,
    };
    let AppInboxOutcome::Accepted(Accepted::New(sequence)) =
        store.app_inbox(accept("occ-1")).unwrap()
    else {
        panic!()
    };
    assert!(matches!(
        store.app_inbox(AppInboxOperation::RemoveRoute {
            owner: "bob".into(),
            installation: "installed".into(),
            route_id: "mail".into(),
        }),
        Err(InboxError::NotFound)
    ));
    store
        .app_inbox(AppInboxOperation::RemoveRoute {
            owner: "alice".into(),
            installation: "installed".into(),
            route_id: "mail".into(),
        })
        .unwrap();
    assert!(matches!(
        store.app_inbox(accept("occ-2")),
        Err(InboxError::NotFound)
    ));
    drop(store);

    let store = DurableKernelStateStore::open_owned(path.clone()).unwrap();
    let AppInboxOutcome::Due(due) = store
        .app_inbox(AppInboxOperation::Due {
            now_ms: 100,
            limit: 10,
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].sequence, sequence);
    assert_eq!(due[0].payload, json!({"text":"kept"}));
    store
        .app_inbox(AppInboxOperation::Delivered {
            sequence,
            generation: 1,
        })
        .unwrap();
    assert_eq!(
        store
            .app_inbox(AppInboxOperation::Due {
                now_ms: 101,
                limit: 10
            })
            .unwrap(),
        AppInboxOutcome::Due(Vec::new())
    );
    store
        .app_inbox(AppInboxOperation::CreateRoute { route, now_ms: 102 })
        .unwrap();
    assert_eq!(
        store.app_inbox(accept("occ-1")).unwrap(),
        AppInboxOutcome::Accepted(Accepted::Duplicate(sequence))
    );
}
