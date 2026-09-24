use super::BrowserControllerProcessStore;
use crate::runtime::browser_controller_action::BrowserLocatorAction;
use crate::transport::room_browser_controller::RoomBrowserControllerResult as Response;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(1);

struct ControllerFixture {
    path: PathBuf,
}

impl ControllerFixture {
    fn new() -> Self {
        Self::with_script(SCRIPT)
    }

    fn with_script(script: &str) -> Self {
        let id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "chariox-browser-request-pipeline-{}-{id}.sh",
            std::process::id()
        ));
        fs::write(&path, script).expect("write fake browser controller");
        let mut permissions = fs::metadata(&path)
            .expect("stat fake browser controller")
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).expect("make fake browser controller executable");
        Self { path }
    }

    fn marker(&self, suffix: &str) -> PathBuf {
        PathBuf::from(format!("{}.{suffix}", self.path.display()))
    }
}

impl Drop for ControllerFixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        for suffix in ["started", "busy", "release", "action", "snapshot", "cancel"] {
            let _ = fs::remove_file(self.marker(suffix));
        }
    }
}

struct ShutdownOnDrop(BrowserControllerProcessStore);

impl Drop for ShutdownOnDrop {
    fn drop(&mut self) {
        let _ = self.0.shutdown();
    }
}

fn wait_for_marker(fixture: &ControllerFixture, suffix: &str) {
    let marker = fixture.marker(suffix);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !marker.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(marker.exists(), "controller did not create {suffix} marker");
}

const SCRIPT: &str = r#"#!/bin/sh
set -eu
first_id=
first_target=
first_document=
snapshot_result() {
  printf '{"id":%s,"ok":true,"result":{"browser_generation":1,"target_id":"%s","document_id":"%s","snapshot_revision":1,"accessibility_nodes":[],"dom_documents":[],"dom_nodes":[]}}\n' "$1" "$2" "$3"
}
while IFS= read -r request; do
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"health"'*)
      printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
    *'"method":"browser.snapshot"'*)
      target=$(printf '%s\n' "$request" | sed -n 's/.*"target_id":"\([^"]*\)".*/\1/p')
      document=$(printf '%s\n' "$request" | sed -n 's/.*"document_id":"\([^"]*\)".*/\1/p')
      if [ -z "$first_id" ]; then
        first_id=$id
        first_target=$target
        first_document=$document
      else
        # Hold the first request until a second tab has reached this process,
        # then deliberately return responses out of order.
        snapshot_result "$id" "$target" "$document"
        snapshot_result "$first_id" "$first_target" "$first_document"
        first_id=
      fi ;;
    *'"method":"shutdown"'*)
      printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"
      exit 0 ;;
  esac
done
"#;

#[test]
fn room_store_pipelines_different_tab_requests_and_demultiplexes_reversed_replies() {
    let fixture = ControllerFixture::new();
    let store = BrowserControllerProcessStore::new(
        &fixture.path,
        Vec::new(),
        Duration::from_secs(3),
    );
    store.acquire("room-pipeline-test").expect("Room acquires controller");

    let barrier = Arc::new(Barrier::new(3));
    let first_store = store.clone();
    let first_barrier = Arc::clone(&barrier);
    let first = thread::spawn(move || {
        first_barrier.wait();
        first_store
            .capture_browser_snapshot("room-pipeline-test", "tab-first", "doc-first")
            .expect("first tab request reaches controller")
            .expect("controller is enabled")
    });
    let second_store = store.clone();
    let second_barrier = Arc::clone(&barrier);
    let second = thread::spawn(move || {
        second_barrier.wait();
        second_store
            .capture_browser_snapshot("room-pipeline-test", "tab-second", "doc-second")
            .expect("second tab request reaches controller")
            .expect("controller is enabled")
    });

    barrier.wait();
    let first = first.join().expect("first caller completes");
    let second = second.join().expect("second caller completes");
    assert_eq!(first.target_id, "tab-first");
    assert_eq!(first.document_id, "doc-first");
    assert_eq!(second.target_id, "tab-second");
    assert_eq!(second.document_id, "doc-second");

    // Per-tab write ordering remains enforced by the controller scheduler;
    // this store is only transport and does not add a second tab authority.
    store.release("room-pipeline-test").expect("Room releases controller");
}

#[test]
fn transient_queued_health_error_does_not_restart_the_controller() {
    let fixture = ControllerFixture::with_script(QUEUED_HEALTH_SCRIPT);
    let store = BrowserControllerProcessStore::new(
        &fixture.path,
        Vec::new(),
        Duration::from_secs(3),
    );
    store.acquire("room-health-reprobe").expect("Room acquires controller");
    let _shutdown = ShutdownOnDrop(store.clone());

    let first_store = store.clone();
    let first = thread::spawn(move || {
        first_store.capture_browser_snapshot("room-health-reprobe", "tab-first", "doc-first")
    });
    wait_for_marker(&fixture, "started");

    let second_store = store.clone();
    let second = thread::spawn(move || {
        second_store.capture_browser_snapshot("room-health-reprobe", "tab-second", "doc-second")
    });
    wait_for_marker(&fixture, "busy");
    fs::write(fixture.marker("release"), "release first snapshot")
        .expect("release the first snapshot barrier");

    let first = first
        .join()
        .expect("first caller joins")
        .expect("first snapshot succeeds")
        .expect("controller is enabled");
    let second = second
        .join()
        .expect("second caller joins")
        .expect("fresh health probe must keep the healthy controller")
        .expect("controller is enabled");
    assert_eq!(first.target_id, "tab-first");
    assert_eq!(second.target_id, "tab-second");
    let snapshot = store
        .snapshot()
        .expect("read controller snapshot")
        .expect("controller is enabled");
    assert_eq!(snapshot.runtime_generation, 1);
    assert_eq!(snapshot.restart_count, 0);
    store.release("room-health-reprobe").expect("release Room lease");
}

#[test]
fn cancellation_fence_reports_the_other_tab_request_as_controller_fenced() {
    const ACTION_ID: &str = "00000000000000000000000000000071";
    let fixture = ControllerFixture::with_script(FENCE_SCRIPT);
    let store = BrowserControllerProcessStore::new(
        &fixture.path,
        Vec::new(),
        Duration::from_millis(500),
    );
    store.acquire("room-fence").expect("Room acquires controller");
    let _shutdown = ShutdownOnDrop(store.clone());

    let action_store = store.clone();
    let action = thread::spawn(move || {
        action_store.perform_cancellable_browser_action(
            "room-fence",
            ACTION_ID,
            "tab-action",
            "doc-action",
            "node-1",
            &BrowserLocatorAction::Click,
            100,
        )
    });
    wait_for_marker(&fixture, "action");
    assert!(store.cancel_browser_action("room-fence", ACTION_ID));
    wait_for_marker(&fixture, "cancel");
    thread::sleep(Duration::from_millis(200));

    let snapshot_store = store.clone();
    let snapshot = thread::spawn(move || {
        snapshot_store.capture_browser_snapshot("room-fence", "tab-second", "doc-second")
    });
    wait_for_marker(&fixture, "snapshot");

    assert_eq!(
        action.join().expect("cancelled action caller joins").unwrap(),
        Response::ActionCancelled {
            controller_fenced: true,
        }
    );
    let snapshot_error = snapshot
        .join()
        .expect("second-tab caller joins")
        .expect_err("the other in-flight request must observe the controller fence");
    assert_eq!(
        snapshot_error,
        "browser controller was fenced after cancellation timed out"
    );
}

const QUEUED_HEALTH_SCRIPT: &str = r#"#!/bin/sh
set -eu
first_snapshot=0
snapshot_result() {
  printf '{"id":%s,"ok":true,"result":{"browser_generation":1,"target_id":"%s","document_id":"%s","snapshot_revision":1,"accessibility_nodes":[],"dom_documents":[],"dom_nodes":[]}}\n' "$1" "$2" "$3"
}
while IFS= read -r request; do
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"health"'*)
      if [ "$first_snapshot" -eq 1 ] && [ ! -e "$0.release" ]; then
        printf '{"id":%s,"ok":false,"error":{"code":"controller_busy","message":"queued behind barrier"}}\n' "$id"
        : > "$0.busy"
      else
        printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$"
      fi ;;
    *'"method":"browser.snapshot"'*)
      target=$(printf '%s\n' "$request" | sed -n 's/.*"target_id":"\([^"]*\)".*/\1/p')
      document=$(printf '%s\n' "$request" | sed -n 's/.*"document_id":"\([^"]*\)".*/\1/p')
      if [ "$first_snapshot" -eq 0 ]; then
        first_snapshot=1
        (
          : > "$0.started"
          while [ ! -e "$0.release" ]; do sleep 0.01; done
          snapshot_result "$id" "$target" "$document"
        ) &
      else
        snapshot_result "$id" "$target" "$document"
      fi ;;
    *'"method":"shutdown"'*)
      printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"
      exit 0 ;;
  esac
done
"#;

const FENCE_SCRIPT: &str = r#"#!/bin/sh
set -eu
while IFS= read -r request; do
  id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$request" in
    *'"method":"health"'*)
      printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
    *'"method":"browser.action"'*) : > "$0.action" ;;
    *'"method":"browser.snapshot"'*) : > "$0.snapshot" ;;
    *'"method":"browser.cancel"'*) : > "$0.cancel" ;;
    *'"method":"shutdown"'*)
      printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"
      exit 0 ;;
  esac
done
"#;
