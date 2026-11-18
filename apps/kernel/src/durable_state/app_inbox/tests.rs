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
        paused: false,
        app_managed: false,
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

struct StoreDir(std::path::PathBuf);
impl Drop for StoreDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn pause_store() -> (StoreDir, DurableKernelStateStore) {
    let dir = StoreDir(std::env::temp_dir().join(format!(
        "chariox-route-pause-{:016x}",
        rand::random::<u64>()
    )));
    std::fs::create_dir(&dir.0).unwrap();
    let store = DurableKernelStateStore::open_owned(dir.0.join("kernel.sqlite")).unwrap();
    crate::durable_state::app_state::fixture_inbox_installation(&store, "alice");
    (dir, store)
}

fn fed_route(id: &str, connection: &str, app_managed: bool) -> InboxRoute {
    InboxRoute {
        route_id: id.into(),
        owner_id: "alice".into(),
        installation_id: "installed".into(),
        event_name: "received".into(),
        source_event_type: "dummy.test".into(),
        source_event_version: 1,
        active: true,
        paused: false,
        app_managed,
        source: Some(chariox_app_runtime::app_inbox::InboxSource {
            generator_id: "dev.chariox.dummy".into(),
            connection_id: connection.into(),
            connection_scope: "default".into(),
            filter_json: "null".into(),
            payload_schema_json: None,
            manifest_digest: None,
        }),
    }
}

fn route_of(store: &DurableKernelStateStore, id: &str) -> Option<InboxRoute> {
    let AppInboxOutcome::Route(route) = store
        .app_inbox(AppInboxOperation::Route {
            owner: "alice".into(),
            installation: "installed".into(),
            route_id: id.into(),
        })
        .unwrap()
    else {
        panic!("route")
    };
    route
}

#[test]
fn a_pause_is_kept_apart_from_the_deployment_handover() {
    let (_dir, store) = pause_store();
    store
        .app_inbox(AppInboxOperation::CreateRoute {
            route: fed_route("mail", "c1", true),
            now_ms: 1,
        })
        .unwrap();
    let accept = |id: &str| AppInboxOperation::Accept {
        owner: "alice".into(),
        installation: "installed".into(),
        route_id: "mail".into(),
        occurrence_id: id.into(),
        payload: json!({"text":"x"}),
        generation: 1,
        now_ms: 100,
    };
    store
        .app_inbox(AppInboxOperation::SetRoutePaused {
            owner: "alice".into(),
            installation: "installed".into(),
            route_id: "mail".into(),
            paused: true,
        })
        .unwrap();
    let paused = route_of(&store, "mail").unwrap();
    assert!((paused.active, paused.paused, paused.admits()) == (true, true, false));
    assert!(matches!(
        store.app_inbox(accept("occ-1")),
        Err(InboxError::NotFound)
    ));
    // A handover that hands the route back (active=true) does not resume it.
    for active in [false, true] {
        store
            .app_inbox(AppInboxOperation::SetRouteActive {
                owner: "alice".into(),
                installation: "installed".into(),
                route_id: "mail".into(),
                active,
            })
            .unwrap();
    }
    assert!(!route_of(&store, "mail").unwrap().admits());
    store
        .app_inbox(AppInboxOperation::SetRoutePaused {
            owner: "alice".into(),
            installation: "installed".into(),
            route_id: "mail".into(),
            paused: false,
        })
        .unwrap();
    assert!(route_of(&store, "mail").unwrap().admits());
    assert!(matches!(
        store.app_inbox(accept("occ-2")),
        Ok(AppInboxOutcome::Accepted(Accepted::New(_)))
    ));
    assert!(matches!(
        store.app_inbox(AppInboxOperation::SetRoutePaused {
            owner: "alice".into(),
            installation: "installed".into(),
            route_id: "missing".into(),
            paused: true,
        }),
        Err(InboxError::NotFound)
    ));
}

#[test]
fn revoking_a_grant_ends_the_routes_the_app_made_and_keeps_the_owners() {
    let (_dir, store) = pause_store();
    for (id, connection, own) in [
        ("app-a", "c1", true),
        ("hand-a", "c1", false),
        ("app-b", "c2", true),
    ] {
        store
            .app_inbox(AppInboxOperation::CreateRoute {
                route: fed_route(id, connection, own),
                now_ms: 1,
            })
            .unwrap();
    }
    for connection in ["c1", "c2"] {
        store
            .app_connection_grant(
                crate::durable_state::app_connections::ConnectionGrantCommand::Grant {
                    owner: "alice".into(),
                    installation: "installed".into(),
                    generator_id: "dev.chariox.dummy".into(),
                    connection_id: connection.into(),
                    event_owner_id: "event-owner".into(),
                    now_ms: 2,
                },
            )
            .unwrap();
    }
    store
        .app_connection_grant(
            crate::durable_state::app_connections::ConnectionGrantCommand::Revoke {
                owner: "alice".into(),
                installation: "installed".into(),
                connection_id: "c1".into(),
            },
        )
        .unwrap();
    assert!(route_of(&store, "app-a").is_none());
    assert!(route_of(&store, "hand-a").is_some());
    assert!(route_of(&store, "app-b").is_some());
}
