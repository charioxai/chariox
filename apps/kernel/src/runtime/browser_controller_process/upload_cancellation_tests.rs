use super::*;
use crate::transport::room_browser_controller::RoomBrowserControllerResult as Response;
use std::fs;

const FIRST: &str = "00000000000000000000000000000001";
const SECOND: &str = "00000000000000000000000000000002";

struct Fixture {
    root: PathBuf,
    store: BrowserControllerProcessStore,
}

impl Fixture {
    fn new(hold: bool) -> Self {
        Self::with_cancel_ack(hold, true)
    }

    fn with_cancel_ack(hold: bool, acknowledge_cancellation: bool) -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-upload-cancel-{:032x}",
            rand::random::<u128>()
        ));
        fs::create_dir(&root).unwrap();
        let script = root.join("controller.sh");
        fs::write(&script, r#"set -eu
root=$1
ack=$2
while IFS= read -r request; do
  id=${request#*:}
  id=${id%%,*}
  printf '%s\n' "$request" >> "$root/requests"
  case "$request" in
    *'"method":"health"'*)
      printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
    *'"method":"browser.upload"'*)
      if [ -f "$root/hold" ]; then
        pending=$id
      else
        printf '{"id":%s,"ok":true,"result":{"browser_generation":1,"target_id":"tab","document_id":"doc","file_count":1,"total_bytes":12}}\n' "$id"
      fi ;;
    *'"method":"browser.cancel"'*)
      if [ "$ack" = yes ]; then
        printf '{"id":%s,"ok":true,"result":{"accepted":true}}\n' "$id"
        printf '{"id":%s,"ok":false,"error":{"code":"browser_action_cancelled","message":"cancelled"}}\n' "$pending"
      fi ;;
    *'"method":"shutdown"'*)
      printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"
      exit 0 ;;
  esac
done
"#).unwrap();
        if hold {
            fs::write(root.join("hold"), "").unwrap();
        }
        let store = BrowserControllerProcessStore::new(
            "/bin/sh",
            vec![
                script.to_string_lossy().into_owned(),
                root.to_string_lossy().into_owned(),
                if acknowledge_cancellation {
                    "yes"
                } else {
                    "no"
                }
                .to_string(),
            ],
            Duration::from_secs(2),
        );
        let fixture = Self { root, store };
        fixture.store.acquire("room").unwrap();
        fixture
    }

    fn files(&self) -> BrowserUploadFiles {
        BrowserUploadFiles::new(vec![self.root.join("report.txt")]).unwrap()
    }

    fn count(&self) -> usize {
        fs::read_to_string(self.root.join("requests"))
            .unwrap_or_default()
            .lines()
            .filter(|line| line.contains("\"method\":\"browser.upload\""))
            .count()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.store.shutdown();
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn completed_upload_recovery_and_duplicates_do_not_resend_files() {
    let fixture = Fixture::new(false);
    let files = fixture.files();
    let result = fixture
        .store
        .perform_cancellable_browser_upload("room", FIRST, "tab", "doc", "backend:1", &files)
        .unwrap();
    assert!(matches!(result, Response::Upload { result: Some(_) }));
    assert_eq!(
        fixture
            .store
            .recover_cancellable_browser_upload("room", FIRST, "tab", "doc", "backend:1", &files)
            .unwrap(),
        result
    );
    assert_eq!(
        fixture
            .store
            .perform_cancellable_browser_upload("room", FIRST, "tab", "doc", "backend:1", &files)
            .unwrap(),
        result
    );
    let changed = BrowserUploadFiles::new(vec![fixture.root.join("other.txt")]).unwrap();
    assert!(fixture
        .store
        .perform_cancellable_browser_upload("room", FIRST, "tab", "doc", "backend:1", &changed)
        .unwrap_err()
        .contains("different request"));
    assert!(fixture
        .store
        .recover_cancellable_browser_upload("room", SECOND, "tab", "doc", "backend:1", &files)
        .unwrap_err()
        .contains("proof is unavailable"));
    assert!(!fixture.store.cancel_browser_action("room", FIRST));
    assert_eq!(fixture.count(), 1);
}

#[test]
fn in_flight_upload_cancels_through_stdio_and_fresh_upload_can_retry() {
    let fixture = Fixture::new(true);
    let files = fixture.files();
    std::thread::scope(|scope| {
        let operation = scope.spawn(|| {
            fixture.store.perform_cancellable_browser_upload(
                "room",
                FIRST,
                "tab",
                "doc",
                "backend:1",
                &files,
            )
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        while fixture.count() == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(fixture.count(), 1);
        assert!(!fixture.store.cancel_browser_action("other-room", FIRST));
        assert!(fixture.store.cancel_browser_action("room", FIRST));
        let result = operation.join().unwrap().unwrap();
        assert_eq!(
            result,
            Response::ActionCancelled {
                controller_fenced: false
            }
        );
        assert_eq!(
            fixture
                .store
                .recover_cancellable_browser_upload(
                    "room",
                    FIRST,
                    "tab",
                    "doc",
                    "backend:1",
                    &files
                )
                .unwrap(),
            result
        );
        let requests: Vec<serde_json::Value> = fs::read_to_string(fixture.root.join("requests"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let upload = requests
            .iter()
            .find(|request| request["method"] == "browser.upload")
            .unwrap();
        let cancel = requests
            .iter()
            .find(|request| request["method"] == "browser.cancel")
            .unwrap();
        assert_eq!(cancel["params"]["request_id"], upload["id"]);
    });
    fs::remove_file(fixture.root.join("hold")).unwrap();
    assert!(matches!(
        fixture
            .store
            .perform_cancellable_browser_upload("room", SECOND, "tab", "doc", "backend:1", &files)
            .unwrap(),
        Response::Upload { result: Some(_) }
    ));
    assert_eq!(fixture.count(), 2);
}

#[test]
fn unacknowledged_upload_cancellation_fences_its_controller() {
    let fixture = Fixture::with_cancel_ack(true, false);
    let files = fixture.files();
    std::thread::scope(|scope| {
        let operation = scope.spawn(|| {
            fixture.store.perform_cancellable_browser_upload(
                "room",
                FIRST,
                "tab",
                "doc",
                "backend:1",
                &files,
            )
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        while fixture.count() == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(fixture.count(), 1);
        assert!(fixture.store.cancel_browser_action("room", FIRST));
        assert_eq!(
            operation.join().unwrap().unwrap(),
            Response::ActionCancelled {
                controller_fenced: true,
            }
        );
    });
    let requests: Vec<serde_json::Value> = fs::read_to_string(fixture.root.join("requests"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let upload = requests
        .iter()
        .find(|request| request["method"] == "browser.upload")
        .unwrap();
    let cancel = requests
        .iter()
        .find(|request| request["method"] == "browser.cancel")
        .unwrap();
    assert_eq!(cancel["params"]["request_id"], upload["id"]);
    assert!(fixture
        .store
        .perform_cancellable_browser_upload("room", SECOND, "tab", "doc", "backend:1", &files)
        .unwrap_err()
        .contains(CONTROLLER_RESTARTED_BEFORE_OPERATION));
    assert_eq!(fixture.count(), 1);
}
