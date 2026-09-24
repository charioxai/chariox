use super::BrowserControllerProcessStore;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(1);

struct ControllerFixture {
    path: PathBuf,
}

impl ControllerFixture {
    fn new() -> Self {
        let id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "chariox-browser-request-pipeline-{}-{id}.sh",
            std::process::id()
        ));
        fs::write(&path, SCRIPT).expect("write fake browser controller");
        let mut permissions = fs::metadata(&path)
            .expect("stat fake browser controller")
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).expect("make fake browser controller executable");
        Self { path }
    }
}

impl Drop for ControllerFixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
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
