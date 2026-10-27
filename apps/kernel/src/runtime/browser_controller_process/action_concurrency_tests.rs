//! Tests physical dispatch overlap, not only concurrent Action ledger timestamps.

use super::*;
use crate::transport::room_browser_controller::RoomBrowserControllerResult as Response;
use std::fs;

const FIRST: &str = "00000000000000000000000000000001";
const SECOND: &str = "00000000000000000000000000000002";

struct CancellationFixture {
    root: PathBuf,
    store: BrowserControllerProcessStore,
}

impl CancellationFixture {
    fn new(acknowledge: bool) -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-action-cancel-{:032x}",
            rand::random::<u128>()
        ));
        fs::create_dir(&root).unwrap();
        let script = root.join("controller.sh");
        fs::write(&script, r#"set -eu
root=$1
ack=$2
first=
while IFS= read -r request; do
  printf '%s\n' "$request" >> "$root/requests"
  id=${request#*:}
  id=${id%%,*}
  case "$request" in
    *'"method":"health"'*)
      printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
    *'"method":"browser.action"'*)
      case "$request" in
        *'"target_id":"first"'*) first=$id; : > "$root/first-started" ;;
        *) printf '{"id":%s,"ok":true,"result":{"browser_generation":1,"target_id":"second","document_id":"doc","action_kind":"click","dialog_opened":false,"attempts":1,"elapsed_ms":1}}\n' "$id" ;;
      esac ;;
    *'"method":"browser.cancel"'*)
      if [ "$ack" = yes ]; then
        printf '{"id":%s,"ok":false,"error":{"code":"browser_action_cancelled","message":"cancelled"}}\n' "$first"
        printf '{"id":%s,"ok":true,"result":{"accepted":true}}\n' "$id"
      fi ;;
    *'"method":"shutdown"'*)
      printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
  esac
done
"#).unwrap();
        let store = BrowserControllerProcessStore::new(
            "/bin/sh",
            vec![
                script.display().to_string(),
                root.display().to_string(),
                if acknowledge { "yes" } else { "no" }.to_string(),
            ],
            Duration::from_millis(500),
        );
        store.acquire("room").unwrap();
        Self { root, store }
    }

    fn start_first(&self) -> std::thread::JoinHandle<Result<Response, String>> {
        let store = self.store.clone();
        let first = std::thread::spawn(move || {
            store.perform_cancellable_browser_action(
                "room",
                FIRST,
                "first",
                "doc",
                "backend:1",
                &BrowserLocatorAction::Click,
                100,
            )
        });
        let deadline = Instant::now() + Duration::from_millis(400);
        while !self.root.join("first-started").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        first
    }
}

impl Drop for CancellationFixture {
    fn drop(&mut self) {
        let _ = self.store.shutdown();
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn cancelling_one_tab_preserves_other_tab_result_and_exact_execution_receipt() {
    let fixture = CancellationFixture::new(true);
    let first = fixture.start_first();
    let second = fixture.store.perform_cancellable_browser_action(
        "room",
        SECOND,
        "second",
        "doc",
        "backend:2",
        &BrowserLocatorAction::Click,
        100,
    );
    // The second physical completion must precede cancellation of the first.
    let cancelled = fixture.store.cancel_browser_action("room", FIRST);
    let first = first.join().unwrap();
    assert!(fixture.root.join("first-started").exists());
    assert!(
        cancelled,
        "exact active execution should accept the cancellation request"
    );
    assert!(matches!(
        second.unwrap(),
        Response::Action { result: Some(_) }
    ));
    assert!(matches!(
        first.unwrap(),
        Response::ActionCancelled {
            controller_fenced: false
        }
    ));
    let requests: Vec<serde_json::Value> = fs::read_to_string(fixture.root.join("requests"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let first = requests
        .iter()
        .find(|request| {
            request["method"] == "browser.action" && request["params"]["target_id"] == "first"
        })
        .unwrap();
    let cancel = requests
        .iter()
        .find(|request| request["method"] == "browser.cancel")
        .unwrap();
    assert_eq!(cancel["params"]["request_id"], first["id"]);
    let count = requests
        .iter()
        .filter(|request| request["method"] == "browser.action")
        .count();
    assert_eq!(count, 2);
    assert!(matches!(
        fixture
            .store
            .recover_cancellable_browser_action(
                "room",
                FIRST,
                "first",
                "doc",
                "backend:1",
                &BrowserLocatorAction::Click,
                100
            )
            .unwrap(),
        Response::ActionCancelled {
            controller_fenced: false
        }
    ));
    assert!(matches!(
        fixture
            .store
            .perform_cancellable_browser_action(
                "room",
                FIRST,
                "first",
                "doc",
                "backend:1",
                &BrowserLocatorAction::Click,
                100
            )
            .unwrap(),
        Response::ActionCancelled {
            controller_fenced: false
        }
    ));
    let after = fs::read_to_string(fixture.root.join("requests")).unwrap();
    assert_eq!(
        after
            .lines()
            .filter(|line| line.contains("\"method\":\"browser.action\""))
            .count(),
        count
    );
}

#[test]
fn unacknowledged_concurrent_action_cancellation_reaps_original_controller() {
    let fixture = CancellationFixture::new(false);
    let first = fixture.start_first();
    let cancelled = fixture.store.cancel_browser_action("room", FIRST);
    let first = first.join().unwrap();
    assert!(fixture.root.join("first-started").exists());
    assert!(
        cancelled,
        "active execution should accept the cancellation request"
    );
    assert!(matches!(
        first.unwrap(),
        Response::ActionCancelled {
            controller_fenced: true
        }
    ));
    assert!(fixture
        .store
        .perform_cancellable_browser_action(
            "room",
            SECOND,
            "second",
            "doc",
            "backend:2",
            &BrowserLocatorAction::Click,
            100
        )
        .unwrap_err()
        .contains(CONTROLLER_RESTARTED_BEFORE_OPERATION));
}

#[test]
fn different_tab_cancellable_actions_overlap_and_match_reversed_replies() {
    let root = std::env::temp_dir().join(format!(
        "chariox-action-overlap-{:032x}",
        rand::random::<u128>()
    ));
    fs::create_dir(&root).unwrap();
    let script = root.join("controller.sh");
    fs::write(&script, r#"set -eu
root=$1
first=
reply() {
  printf '{"id":%s,"ok":true,"result":{"browser_generation":1,"target_id":"%s","document_id":"doc","action_kind":"click","dialog_opened":false,"attempts":1,"elapsed_ms":1}}\n' "$1" "$2"
}
while IFS= read -r request; do
  id=${request#*:}
  id=${id%%,*}
  case "$request" in
    *'"method":"health"'*)
      printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
    *'"method":"browser.action"'*)
      if [ -z "$first" ]; then
        first=$id
        : > "$root/first-started"
      else
        reply "$id" second
        reply "$first" first
      fi ;;
    *'"method":"shutdown"'*)
      printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"
      exit 0 ;;
  esac
done
"#).unwrap();
    let store = BrowserControllerProcessStore::new(
        "/bin/sh",
        vec![script.display().to_string(), root.display().to_string()],
        Duration::from_secs(2),
    );
    store.acquire("room").unwrap();
    let first_store = store.clone();
    let first = std::thread::spawn(move || {
        first_store.perform_cancellable_browser_action(
            "room",
            "00000000000000000000000000000001",
            "first",
            "doc",
            "backend:1",
            &BrowserLocatorAction::Click,
            1000,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(1);
    while !root.join("first-started").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    // Join and clean up before assertions, including on a serialization failure.
    let started = root.join("first-started").exists();
    let second_store = store.clone();
    let second = std::thread::spawn(move || {
        second_store.perform_cancellable_browser_action(
            "room",
            "00000000000000000000000000000002",
            "second",
            "doc",
            "backend:2",
            &BrowserLocatorAction::Click,
            1000,
        )
    });
    let first = first.join().unwrap();
    let second = second.join().unwrap();
    let _ = store.shutdown();
    fs::remove_dir_all(&root).unwrap();
    assert!(started, "first action must physically reach the controller");
    for (response, target) in [(first, "first"), (second, "second")] {
        match response.expect("independent-tab actions must overlap before either can finish") {
            Response::Action {
                result: Some(result),
            } => assert_eq!(result.target_id, target),
            other => panic!("expected matching action result, got {other:?}"),
        }
    }
}
