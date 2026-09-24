use super::*;
use crate::runtime::browser_import_payload::BrowserImportPayload;
use crate::transport::room_browser_controller::{
    RoomBrowserControllerResult as Response, RoomBrowserImportBinding,
};
use std::fs;
use zeroize::Zeroizing;

const REQUEST: &str = "00000000000000000000000000000041";

#[test]
fn delayed_import_cancels_exact_execution_through_stdio_after_rollback() {
    let root = std::env::temp_dir().join(format!(
        "chariox-import-cancel-{:032x}",
        rand::random::<u128>()
    ));
    fs::create_dir(&root).unwrap();
    let script = root.join("controller.sh");
    fs::write(
        &script,
        r#"set -eu
root=$1
while IFS= read -r request; do
  id=${request#*:}
  id=${id%%,*}
  case "$request" in
    *'"method":"health"'*)
      printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
    *'"method":"browser.cookies.import"'*)
      printf '%s\n' "$request" >> "$root/import-requests"
      pending=$id ;;
    *'"method":"browser.cancel"'*)
      printf '%s\n' "$request" >> "$root/cancel-requests"
      printf '{"id":%s,"ok":true,"result":{"status":"rolled_back","results":[]}}\n' "$pending"
      printf '{"id":%s,"ok":true,"result":{"accepted":true}}\n' "$id" ;;
    *'"method":"shutdown"'*)
      printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"
      exit 0 ;;
  esac
done
"#,
    )
    .unwrap();
    let store = BrowserControllerProcessStore::new(
        "/bin/sh",
        vec![
            script.to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
        ],
        Duration::from_secs(2),
    );
    store.acquire("room").unwrap();
    let binding = RoomBrowserImportBinding {
        request_id: REQUEST.into(),
        user_id: "user".into(),
        room_id: "room".into(),
        environment_id: "environment".into(),
    };
    let payload = BrowserImportPayload::new(Zeroizing::new("[]".into())).unwrap();
    std::thread::scope(|scope| {
        let operation = scope.spawn(|| {
            store.perform_cancellable_browser_import(
                "room",
                &binding,
                1,
                "target",
                "document",
                "0",
                &["example.test".into()],
                &[],
                false,
                &payload,
            )
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        while !root.join("import-requests").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(root.join("import-requests").exists());
        assert!(!store.cancel_browser_import_and_wait("room", "00000000000000000000000000000042"));
        assert!(store.cancel_browser_import_and_wait("room", REQUEST));
        assert_eq!(
            operation.join().unwrap().unwrap(),
            Response::ActionCancelled {
                controller_fenced: false
            }
        );
    });
    let cancelled = fs::read_to_string(root.join("cancel-requests")).unwrap();
    assert!(cancelled.contains(r#""method":"browser.cancel""#));
    assert!(cancelled.contains(r#""request_id":2"#));
    store.shutdown().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancellation_before_registration_fences_the_later_import_before_stdio_dispatch() {
    let root = std::env::temp_dir().join(format!(
        "chariox-import-cancel-before-register-{:032x}",
        rand::random::<u128>()
    ));
    fs::create_dir(&root).unwrap();
    let script = root.join("controller.sh");
    fs::write(
        &script,
        r#"set -eu
root=$1
while IFS= read -r request; do
  id=${request#*:}; id=${id%%,*}
  case "$request" in
    *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
    *'"method":"browser.cookies.import"'*) printf '%s\n' "$request" > "$root/unexpected-import"; printf '{"id":%s,"ok":true,"result":{"status":"applied","results":[]}}\n' "$id" ;;
    *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
  esac
done
"#,
    )
    .unwrap();
    let store = BrowserControllerProcessStore::new(
        "/bin/sh",
        vec![
            script.to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
        ],
        Duration::from_secs(2),
    );
    store.acquire("room").unwrap();
    store.plan_browser_import("room", REQUEST).unwrap();
    let binding = RoomBrowserImportBinding {
        request_id: REQUEST.into(),
        user_id: "user".into(),
        room_id: "room".into(),
        environment_id: "environment".into(),
    };
    let payload = BrowserImportPayload::new(Zeroizing::new("[]".into())).unwrap();
    std::thread::scope(|scope| {
        let cancellation = scope.spawn(|| store.cancel_browser_import_and_wait("room", REQUEST));
        std::thread::sleep(Duration::from_millis(30));
        let result = store
            .perform_cancellable_browser_import(
                "room",
                &binding,
                1,
                "target",
                "document",
                "0",
                &["example.test".into()],
                &[],
                false,
                &payload,
            )
            .unwrap();
        assert_eq!(
            result,
            Response::ActionCancelled {
                controller_fenced: false,
            }
        );
        assert!(cancellation.join().unwrap());
    });
    assert!(!root.join("unexpected-import").exists());
    store.shutdown().unwrap();
    fs::remove_dir_all(root).unwrap();
}
