//! MD-3/MD-5: exercise the actual Rust stdio cancellation path, without Chromium.
use super::*;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
fn md3_host_input_and_md5_vault_input_cancel_across_stdio() {
    let root = std::env::temp_dir().join(format!(
        "chariox-host-cancel-{:032x}",
        rand::random::<u128>()
    ));
    fs::create_dir(&root).unwrap();
    let script = root.join("controller.sh");
    fs::write(&script, r#"set -eu
root=$1
while IFS= read -r request; do
  id=${request#*:}; id=${id%%,*}
  case "$request" in
    *'"method":"health"'*)
      printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
    *'"method":"host.browser"'*|*'"method":"host.secret"'*)
      pending=$id; printf '%s\n' "$request" >> "$root/requests" ;;
    *'"method":"browser.cancel"'*)
      printf '%s\n' "$request" >> "$root/requests"
      printf '{"id":%s,"ok":true,"result":{"accepted":true}}\n' "$id"
      printf '{"id":%s,"ok":false,"error":{"code":"browser_action_cancelled","message":"cancelled"}}\n' "$pending" ;;
    *'"method":"shutdown"'*)
      printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
  esac
done
"#).unwrap();
    let mut backend = BrowserControllerProcessStdioBackend::new(
        "/bin/sh",
        vec![script.display().to_string(), root.display().to_string()],
        Duration::from_secs(2),
    );
    backend.start().unwrap();
    for method in ["host.browser", "host.secret"] {
        fs::remove_file(root.join("requests")).ok();
        let focused = Arc::new(BrowserCancellation::default());
        let authority = Arc::new(AtomicBool::new(true));
        let check = authority.clone();
        let signal = Arc::new(BrowserCancellation::for_authority(
            focused.clone(),
            move || check.load(Ordering::Acquire),
        ));
        std::thread::scope(|scope| {
            let pending = scope.spawn(|| {
                backend.host_request_cancellable(
                    method,
                    serde_json::json!({"operation":"input"}),
                    Some(signal.clone()),
                )
            });
            let deadline = Instant::now() + Duration::from_secs(2);
            while !root.join("requests").exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(root.join("requests").exists());
            if method == "host.browser" {
                focused.request_cancel();
            } else {
                authority.store(false, Ordering::Release);
            }
            assert!(pending
                .join()
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("browser_action_cancelled"));
        });
        let requests: Vec<serde_json::Value> = fs::read_to_string(root.join("requests"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1]["params"]["request_id"], requests[0]["id"]);
        assert!(signal.requested());
        // An already-revoked interval never reaches the controller a second time.
        assert!(backend
            .host_request_cancellable(method, serde_json::json!({}), Some(signal))
            .unwrap_err()
            .to_string()
            .contains("before dispatch"));
        assert_eq!(
            fs::read_to_string(root.join("requests"))
                .unwrap()
                .lines()
                .count(),
            2
        );
    }
    backend.stop().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn md_display_host_crash_reaps_descendant_spawned_after_startup() {
    fn alive(pid: u32) -> bool {
        fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .is_some_and(|s| {
                s.rsplit_once(')')
                    .is_some_and(|(_, tail)| !tail.trim_start().starts_with('Z'))
            })
    }
    let root = std::env::temp_dir().join(format!(
        "chariox-md-host-orphan-{:032x}",
        rand::random::<u128>()
    ));
    fs::create_dir(&root).unwrap();
    let script = root.join("host.sh");
    fs::write(&script,r#"set -eu
root=$1
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 case "$request" in
 *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s}}\n' "$id" "$$" ;;
 *'"method":"host.browser"'*) sleep 30 & printf '%s' "$!" > "$root/survivor"; printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
 *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id"; exit 0 ;;
 esac
done
"#).unwrap();
    let mut backend = BrowserControllerProcessStdioBackend::new(
        "/bin/sh",
        vec![script.display().to_string(), root.display().to_string()],
        Duration::from_secs(2),
    )
    .for_host();
    backend.start().unwrap();
    backend
        .host_request("host.browser", serde_json::json!({"op":"open"}))
        .unwrap();
    let survivor: u32 = fs::read_to_string(root.join("survivor"))
        .unwrap()
        .parse()
        .unwrap();
    assert!(survivor > 1 && alive(survivor));
    // Independent witness cleans a failing test; it does not refresh production ownership.
    let process = backend.process.as_mut().unwrap();
    let mut witness =
        super::owned_process_group::OwnedProcessGroup::new(process.child.lock().unwrap().id());
    {
        let mut child = process.child.lock().unwrap();
        assert!(child.id() > 1);
        child.kill().unwrap();
        child.wait().unwrap();
    }
    backend.take_exited_process().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while alive(survivor) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let leaked = alive(survivor);
    witness.signal();
    backend.stop().unwrap();
    fs::remove_dir_all(root).unwrap();
    assert!(
        !leaked,
        "MD-DISPLAY: supervisor death left a live owned descendant"
    );
}
