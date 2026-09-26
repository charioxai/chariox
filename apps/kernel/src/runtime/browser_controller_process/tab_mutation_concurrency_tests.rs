use super::*;
use crate::runtime::browser_controller_compatibility::BrowserNavigationUrl;
use crate::runtime::browser_import_payload::BrowserImportPayload;
use crate::transport::room_browser_controller::{
    BrowserLifecycleOperation as Operation, RoomBrowserControllerResult as Response,
    RoomBrowserImportBinding,
};
use std::fs;
use std::sync::mpsc;
use zeroize::Zeroizing;

const FIRST: &str = "00000000000000000000000000000001";
const SAME_TAB: &str = "00000000000000000000000000000002";
const OTHER_TAB: &str = "00000000000000000000000000000003";

struct Fixture {
    root: PathBuf,
    store: BrowserControllerProcessStore,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-tab-mutation-overlap-{:032x}",
            rand::random::<u128>()
        ));
        fs::create_dir(&root).unwrap();
        let script = root.join("controller.sh");
        fs::write(
            &script,
            r#"set -eu
root=$1
reply() {
  printf '{"id":%s,"ok":true,"result":{"browser_generation":1,"target_id":"%s","document_id":"doc-%s","url":"https://example.test/%s"}}\n' "$1" "$2" "$2" "$3"
}
while IFS= read -r request; do
  printf '%s\n' "$request" >> "$root/requests"
  id=${request#*:}
  id=${id%%,*}
  case "$request" in
    *'"method":"health"'*)
      printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
    *'"method":"browser.navigate"'*'"url":"https://example.test/first"'*)
      : > "$root/first-a-started"
      (
        while [ ! -f "$root/release-a" ]; do sleep 0.01; done
        : > "$root/first-a-terminal"
        reply "$id" tab-a first
      ) & ;;
    *'"method":"browser.navigate"'*'"url":"https://example.test/second"'*)
      : > "$root/second-a-dispatched"
      if [ ! -f "$root/first-a-terminal" ]; then : > "$root/same-tab-overtook"; fi
      reply "$id" tab-a second ;;
    *'"method":"browser.navigate"'*'"target_id":"tab-b"'*)
      : > "$root/tab-b-dispatched"
      reply "$id" tab-b other ;;
    *'"method":"browser.cookies.import"'*)
      : > "$root/import-dispatched"
      printf '{"id":%s,"ok":true,"result":{"status":"rolled_back","results":[]}}\n' "$id" ;;
    *'"method":"shutdown"'*)
      : > "$root/shutdown-dispatched"
      printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"
      exit 0 ;;
  esac
done
"#,
        )
        .unwrap();
        let store = BrowserControllerProcessStore::new(
            "/bin/sh",
            vec![script.display().to_string(), root.display().to_string()],
            Duration::from_secs(2),
        );
        store.acquire("room").unwrap();
        Self { root, store }
    }

    fn wait_for(&self, name: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.root.join(name).exists() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.root.join(name).exists()
    }
}

fn navigate(
    store: BrowserControllerProcessStore,
    execution_id: &'static str,
    target: &'static str,
    path: &'static str,
) -> Result<Response, String> {
    store.perform_cancellable_browser_lifecycle(
        "room",
        execution_id,
        target,
        if target == "tab-a" {
            "doc-tab-a"
        } else {
            "doc-tab-b"
        },
        &Operation::Navigate {
            url: BrowserNavigationUrl::new(&format!("https://example.test/{path}")).unwrap(),
        },
    )
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::write(self.root.join("release-a"), "");
        let _ = self.store.shutdown();
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn held_navigation_on_one_tab_allows_other_tab_and_orders_same_tab() {
    let fixture = Fixture::new();
    let (other_tx, other_rx) = mpsc::channel();

    let first_store = fixture.store.clone();
    let first = std::thread::spawn(move || navigate(first_store, FIRST, "tab-a", "first"));
    assert!(fixture.wait_for("first-a-started", Duration::from_secs(1)));

    let other_store = fixture.store.clone();
    let other = std::thread::spawn(move || {
        let result = navigate(other_store, OTHER_TAB, "tab-b", "other");
        other_tx.send(result.is_ok()).unwrap();
        result
    });
    let same_tab_store = fixture.store.clone();
    let same_tab =
        std::thread::spawn(move || navigate(same_tab_store, SAME_TAB, "tab-a", "second"));

    let other_completed_while_first_was_held =
        other_rx.recv_timeout(Duration::from_millis(500)).ok() == Some(true);
    let same_tab_overtook = fixture.wait_for("same-tab-overtook", Duration::from_millis(100));
    fs::write(fixture.root.join("release-a"), "").unwrap();

    let first = first.join().unwrap();
    let same_tab = same_tab.join().unwrap();
    let other = other.join().unwrap();

    assert!(
        other_completed_while_first_was_held,
        "tab B must complete while tab A navigation is still held"
    );
    assert!(
        fixture.root.join("tab-b-dispatched").exists(),
        "tab B must reach the physical stdio controller before tab A is released"
    );
    assert!(
        !same_tab_overtook,
        "a second mutation for tab A must wait for the first tab A result"
    );
    for (result, target, path) in [
        (first, "tab-a", "first"),
        (same_tab, "tab-a", "second"),
        (other, "tab-b", "other"),
    ] {
        match result.unwrap() {
            Response::Navigation {
                result: Some(result),
            } => {
                assert_eq!(result.target_id, target);
                assert_eq!(result.url, format!("https://example.test/{path}"));
            }
            other => panic!("expected navigation result, got {other:?}"),
        }
    }
}

#[test]
fn global_release_waits_for_a_tab_mutation_terminal_result() {
    let fixture = Fixture::new();
    let first_store = fixture.store.clone();
    let first = std::thread::spawn(move || navigate(first_store, FIRST, "tab-a", "first"));
    assert!(fixture.wait_for("first-a-started", Duration::from_secs(1)));

    let release_store = fixture.store.clone();
    let release = std::thread::spawn(move || release_store.release("room"));
    let shutdown_overtook = fixture.wait_for("shutdown-dispatched", Duration::from_millis(150));
    fs::write(fixture.root.join("release-a"), "").unwrap();

    assert!(matches!(
        first.join().unwrap().unwrap(),
        Response::Navigation { result: Some(_) }
    ));
    release.join().unwrap().unwrap();
    assert!(
        !shutdown_overtook,
        "controller release must wait until the held tab mutation is terminal"
    );
    assert!(fixture.root.join("shutdown-dispatched").exists());
}

#[test]
fn global_import_waits_for_a_tab_mutation_and_keeps_rollback_receipt() {
    let fixture = Fixture::new();
    let first_store = fixture.store.clone();
    let first = std::thread::spawn(move || navigate(first_store, FIRST, "tab-a", "first"));
    assert!(fixture.wait_for("first-a-started", Duration::from_secs(1)));

    let binding = RoomBrowserImportBinding {
        request_id: "00000000000000000000000000000004".into(),
        user_id: "user".into(),
        room_id: "room".into(),
        environment_id: "environment".into(),
    };
    let payload = BrowserImportPayload::new(Zeroizing::new("[]".into())).unwrap();
    let import_store = fixture.store.clone();
    let import = std::thread::spawn(move || {
        import_store.perform_cancellable_browser_import(
            "room",
            &binding,
            1,
            "tab-a",
            "doc-tab-a",
            "source-store",
            &["example.test".into()],
            &[],
            false,
            &payload,
        )
    });
    let import_overtook = fixture.wait_for("import-dispatched", Duration::from_millis(150));
    fs::write(fixture.root.join("release-a"), "").unwrap();

    assert!(matches!(
        first.join().unwrap().unwrap(),
        Response::Navigation { result: Some(_) }
    ));
    assert_eq!(
        import.join().unwrap().unwrap(),
        Response::CookieImportRolledBack
    );
    assert!(
        !import_overtook,
        "cookie import must keep the global barrier until tab mutation is terminal"
    );
    assert!(fixture.root.join("import-dispatched").exists());
}

#[test]
fn lifecycle_recovery_and_duplicate_execution_replay_without_redispatch() {
    let fixture = Fixture::new();
    let operation = Operation::Navigate {
        url: BrowserNavigationUrl::new("https://example.test/other").unwrap(),
    };
    let first = fixture
        .store
        .perform_cancellable_browser_lifecycle("room", OTHER_TAB, "tab-b", "doc-tab-b", &operation)
        .unwrap();
    let recovered = fixture
        .store
        .recover_cancellable_browser_lifecycle("room", OTHER_TAB, "tab-b", "doc-tab-b", &operation)
        .unwrap();
    let duplicate = fixture
        .store
        .perform_cancellable_browser_lifecycle("room", OTHER_TAB, "tab-b", "doc-tab-b", &operation)
        .unwrap();

    assert_eq!(recovered, first);
    assert_eq!(duplicate, first);
    let navigation_count = fs::read_to_string(fixture.root.join("requests"))
        .unwrap()
        .lines()
        .filter(|line| line.contains("\"method\":\"browser.navigate\""))
        .count();
    assert_eq!(navigation_count, 1);
}
