//! NC-14 / NC-15: a watcher is a notification source whose run commits its events, cursor,
//! reading and health in one write of the real durable writer.
use super::outside_agent_tests::{accept, append, events, follow};
use super::tests::{cleanup, Fixture};
use super::watcher::{
    RunCommit, WatcherOperation, WatcherRecord, WatcherWritten, RATE_LIMIT, SLOWED_GAP_MS,
};
use super::*;
use crate::local::{
    NotificationSourceKind, OutsideEventValue, WatcherCoalesce, WatcherDryRun, WatcherEvent,
    WatcherEventDeclaration, WatcherHealth, WatcherManifest, WatcherMode, WatcherScript,
    WatcherState,
};

const WATCHER: &str = "w1";

fn manifest(coalesce: WatcherCoalesce, debounce_s: u64) -> WatcherManifest {
    WatcherManifest {
        name: "Build status".into(),
        sentence: "Tells you when the nightly build turns red.".into(),
        mode: WatcherMode::Poll { every_s: 60 },
        command: vec!["sh".into(), "watch.sh".into()],
        events: vec![WatcherEventDeclaration {
            name: "red".into(),
            keyed: true,
        }],
        coalesce,
        debounce_s,
    }
}
fn create(f: &Fixture, coalesce: WatcherCoalesce, debounce_s: u64) -> WatcherRecord {
    let record = WatcherRecord {
        watcher_id: WATCHER.into(),
        owner_id: "local".into(),
        source_id: format!("watcher_{WATCHER}"),
        kernel_id: "daemon-test".into(),
        session_id: f.session.clone(),
        creator_agent_id: Some("agent-1".into()),
        manifest: manifest(coalesce, debounce_s),
        script: WatcherScript {
            file_name: "watch.sh".into(),
            text: "true".into(),
        },
        state: WatcherState::Draft,
        health: WatcherHealth::Ok,
        revision: 1,
        dry_run: None,
        cursor: None,
        reading: None,
        last_check_ms: None,
        last_ok_ms: None,
        active_since_ms: None,
        last_fired_ms: None,
        fired_count: 0,
        fired_window: Vec::new(),
        held: Vec::new(),
        held_release_ms: None,
        last_failure_output: None,
        created_ms: 1,
    };
    f.store
        .watcher_write(WatcherOperation::Create(Box::new(record)))
        .unwrap()
        .record
}
fn write(f: &Fixture, operation: WatcherOperation) -> WatcherWritten {
    f.store.watcher_write(operation).unwrap()
}
fn activate(f: &Fixture) {
    pass_dry_run(f);
    write(f, set_state(WatcherState::Active, 1));
}
fn pass_dry_run(f: &Fixture) {
    write(
        f,
        WatcherOperation::DryRunPassed {
            owner: "local".into(),
            watcher_id: WATCHER.into(),
            revision: 1,
            result: WatcherDryRun {
                reading: None,
                history: true,
                events: Vec::new(),
            },
        },
    );
}
fn set_state(state: WatcherState, at: u64) -> WatcherOperation {
    WatcherOperation::SetState {
        owner: "local".into(),
        watcher_id: WATCHER.into(),
        state,
        at,
    }
}
fn event(key: &str, text: &str) -> WatcherEvent {
    WatcherEvent {
        event: "red".into(),
        key: Some(key.into()),
        text: text.into(),
    }
}
fn commit(run_id: &str, events: Vec<WatcherEvent>, cursor: u64, at: u64) -> WatcherOperation {
    WatcherOperation::Commit(RunCommit {
        watcher_id: WATCHER.into(),
        revision: 1,
        run_id: run_id.into(),
        reading: Some(format!("{} failing", events.len())),
        cursor: Some(serde_json::json!({ "seen": cursor })),
        events,
        at,
    })
}
/// Delivery reads occurrences that are due and not yet expired by the real clock, so delivered
/// events happen a minute ago.
fn minute_ago() -> u64 {
    crate::session::unix_epoch_ms() - 60_000
}
fn source(f: &Fixture) -> WorkflowNotificationSource {
    f.store
        .notification_inventory("local")
        .unwrap()
        .0
        .into_iter()
        .find(|source| source.kind == NotificationSourceKind::Watcher)
        .unwrap()
}
fn delivered(f: &Fixture, sub: &WorkflowNotificationSubscription) -> Vec<String> {
    for (_, env) in f.candidates(false) {
        accept(f, sub, &env).unwrap();
        append(f, sub, &env);
    }
    events(f)
        .into_iter()
        .map(|event| {
            let OutsideEventValue::Notification {
                source_kind,
                output,
                ..
            } = event.value
            else {
                panic!("expected a notification, got {event:?}");
            };
            assert_eq!(source_kind, NotificationSourceKind::Watcher);
            output.message().to_owned()
        })
        .collect()
}

#[test]
fn a_watcher_is_a_source_that_delivers_each_committed_event_once_with_its_cursor() {
    let f = Fixture::new();
    create(&f, WatcherCoalesce::Every, 0);
    let source = source(&f);
    assert!(source.enabled);
    assert_eq!(source.display_name, "Build status");
    let sub = follow(&f, &source);
    activate(&f);
    let t = minute_ago();
    let written = write(
        &f,
        commit("run-a.1", vec![event("main", "main is red")], 7, t),
    );
    assert_eq!(
        written.record.cursor,
        Some(serde_json::json!({ "seen": 7 }))
    );
    assert_eq!(written.record.reading.as_deref(), Some("1 failing"));
    assert_eq!(written.record.fired_count, 1);
    // A commit the kernel already took (a repeat after a crash) delivers nothing again.
    write(
        &f,
        commit("run-a.1", vec![event("main", "main is red")], 7, t + 1),
    );
    assert_eq!(delivered(&f, &sub), vec!["main is red".to_owned()]);
    cleanup(f);
}

#[test]
fn turning_a_watcher_on_needs_a_dry_run_of_its_current_version() {
    let f = Fixture::new();
    create(&f, WatcherCoalesce::Every, 0);
    let refused = f.store.watcher_write(set_state(WatcherState::Active, 1));
    assert!(refused.unwrap_err().to_string().contains("dry run"));
    activate(&f);
    write(
        &f,
        WatcherOperation::Change {
            owner: "local".into(),
            watcher_id: WATCHER.into(),
            manifest: manifest(WatcherCoalesce::Every, 0),
            script: WatcherScript {
                file_name: "watch.sh".into(),
                text: "echo changed".into(),
            },
        },
    );
    let changed = f.store.watcher(WATCHER).unwrap().unwrap();
    assert_eq!(changed.state, WatcherState::Draft);
    assert_eq!(changed.summary().dry_run, None);
    assert!(f
        .store
        .watcher_write(set_state(WatcherState::Active, 2))
        .is_err());
    // A run that started under the old version commits nothing.
    assert!(f
        .store
        .watcher_write(commit("old", vec![event("main", "stale")], 1, 3))
        .is_err());
    cleanup(f);
}

#[test]
fn naming_a_watcher_keeps_its_dry_run_and_renames_its_source() {
    let f = Fixture::new();
    create(&f, WatcherCoalesce::Every, 0);
    pass_dry_run(&f);
    write(
        &f,
        WatcherOperation::Rename {
            owner: "local".into(),
            watcher_id: WATCHER.into(),
            name: "Nightly build".into(),
        },
    );
    let named = f.store.watcher(WATCHER).unwrap().unwrap();
    assert_eq!(
        (named.revision, named.manifest.name.as_str()),
        (1, "Nightly build")
    );
    assert!(named.summary().dry_run.is_some());
    assert_eq!(source(&f).display_name, "Nightly build");
    // Turning it on still counts the dry run it passed before the rename.
    write(&f, set_state(WatcherState::Active, 2));
    cleanup(f);
}

#[test]
fn a_paused_or_removed_watcher_delivers_nothing_and_removal_disables_its_source() {
    let f = Fixture::new();
    create(&f, WatcherCoalesce::Every, 0);
    let sub = follow(&f, &source(&f));
    activate(&f);
    write(&f, set_state(WatcherState::Paused, 2));
    assert!(f
        .store
        .watcher_write(commit("run", vec![event("main", "paused")], 1, 3))
        .is_err());
    write(
        &f,
        WatcherOperation::Remove {
            owner: "local".into(),
            watcher_id: WATCHER.into(),
        },
    );
    assert!(f.store.watcher(WATCHER).unwrap().is_none());
    assert!(!source(&f).enabled);
    assert!(delivered(&f, &sub).is_empty());
    cleanup(f);
}

#[test]
fn latest_per_key_and_debounce_hold_events_until_quiet_and_keep_each_keys_newest() {
    let f = Fixture::new();
    create(&f, WatcherCoalesce::LatestPerKey, 30);
    let sub = follow(&f, &source(&f));
    activate(&f);
    let t = minute_ago();
    write(
        &f,
        commit(
            "r1",
            vec![event("main", "main red 1"), event("dev", "dev red")],
            1,
            t + 1_000,
        ),
    );
    write(
        &f,
        commit("r2", vec![event("main", "main red 2")], 2, t + 20_000),
    );
    // Still inside the quiet period of the last event.
    write(
        &f,
        WatcherOperation::Release {
            watcher_id: WATCHER.into(),
            at: t + 40_000,
        },
    );
    assert!(f.candidates(false).is_empty());
    write(
        &f,
        WatcherOperation::Release {
            watcher_id: WATCHER.into(),
            at: t + 50_000,
        },
    );
    assert_eq!(
        delivered(&f, &sub),
        vec!["dev red".to_owned(), "main red 2".to_owned()]
    );
    cleanup(f);
}

#[test]
fn a_source_over_the_rate_limit_turns_noisy_once_and_then_fires_one_held_event_a_minute() {
    let f = Fixture::new();
    create(&f, WatcherCoalesce::Every, 0);
    activate(&f);
    let burst = (0..RATE_LIMIT + 5)
        .map(|n| event(&format!("k{n}"), &format!("event {n}")))
        .collect();
    let written = write(&f, commit("burst", burst, 1, 1_000));
    assert_eq!(
        written.entered,
        Some(WatcherHealth::Noisy { since_ms: 1_000 })
    );
    assert_eq!(written.record.fired_count, RATE_LIMIT as u64);
    assert_eq!(written.record.held.len(), 5);
    let more = write(&f, commit("more", vec![event("k99", "later")], 2, 2_000));
    assert_eq!(more.entered, None, "a noisy source is reported once");
    assert_eq!(more.record.held.len(), 6);
    let released = write(
        &f,
        WatcherOperation::Release {
            watcher_id: WATCHER.into(),
            at: 1_000 + SLOWED_GAP_MS,
        },
    );
    assert_eq!(released.record.fired_count, RATE_LIMIT as u64 + 1);
    assert_eq!(released.record.held.len(), 5);
    cleanup(f);
}

#[test]
fn a_failing_run_is_reported_once_and_a_good_run_clears_it() {
    let f = Fixture::new();
    create(&f, WatcherCoalesce::Every, 0);
    activate(&f);
    let failed = |at| WatcherOperation::Failed {
        watcher_id: WATCHER.into(),
        revision: 1,
        reason: "exit status 2".into(),
        output: Some("curl: (6) Could not resolve host".into()),
        at,
    };
    let first = write(&f, failed(10));
    assert_eq!(
        first.entered,
        Some(WatcherHealth::Failing {
            since_ms: 10,
            reason: "exit status 2".into()
        })
    );
    assert_eq!(write(&f, failed(20)).entered, None);
    let good = write(&f, commit("ok", Vec::new(), 1, 30));
    assert_eq!(good.record.health, WatcherHealth::Ok);
    assert_eq!(good.record.last_failure_output, None);
    let gap = write(
        &f,
        WatcherOperation::Gap {
            watcher_id: WATCHER.into(),
            at: 400,
        },
    );
    assert_eq!(gap.entered, Some(WatcherHealth::Gap { since_ms: 400 }));
    cleanup(f);
}
