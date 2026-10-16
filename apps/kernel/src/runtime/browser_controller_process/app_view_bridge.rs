//! Dispatch bridge drains/replies under ownership, then wait without its lock.
use super::*;
use crate::runtime::browser_controller_app_view::BrowserAppViewRequest;

impl BrowserControllerProcessStore {
    /// MD-N2 / MP-08: observations use the existing unlocked CDP response path.
    pub(crate) fn note_observation(
        &self,
        session_id: &str,
        target_id: &str,
        document_id: &str,
        quote: Option<&crate::local::NoteTextQuote>,
    ) -> Result<Option<crate::runtime::notes::observation::BrowserNoteObservation>, String> {
        if let Some(quote) = quote {
            crate::runtime::notes::validate_quote(quote)?;
        }
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let (pending, timeout) = {
            let mut ownership = ownership
                .lock()
                .map_err(|_| "MD-N2: controller lock unavailable")?;
            ownership.require_lease(session_id)?;
            let supervisor = &mut ownership.supervisor;
            supervisor.prepare_unlocked_request()?;
            (supervisor.backend.begin_observation_request("browser.notes.observe", serde_json::json!({"target_id":target_id,"document_id":document_id,"quote":quote}))?, supervisor.backend.timeout)
        };
        let observation: crate::runtime::notes::observation::BrowserNoteObservation = pending
            .wait(timeout)?
            .into_result("browser.notes.observe")?;
        observation.validate(Some(target_id), Some(document_id))?;
        Ok(Some(observation))
    }

    pub(super) fn app_view_bridge(
        &self,
        session_id: &str,
        request: &BrowserAppViewRequest,
    ) -> Result<Option<serde_json::Value>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        // Replies remain serialized as before, without occupying ownership or
        // filling the shared response registry while they wait for dispatch.
        let _reply = if matches!(request, BrowserAppViewRequest::Respond { .. }) {
            Some(
                self.app_view_replies
                    .lock()
                    .map_err(|_| "App view reply lock poisoned")?,
            )
        } else {
            None
        };
        let (pending, timeout) = {
            let mut ownership = ownership
                .lock()
                .map_err(|_| "browser controller supervisor lock poisoned")?;
            self.authorize()?;
            ownership.require_app_view_lease(session_id)?;
            let supervisor = &mut ownership.supervisor;
            supervisor.prepare_unlocked_request()?;
            (
                supervisor
                    .backend
                    .begin_observation_request(request.method(), request.params())?,
                supervisor.backend.timeout,
            )
        };
        pending
            .wait(timeout)
            .map_err(|error| format!("{}: {error}", request.method()))?
            .into_result(request.method())
            .map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::TestTool;
    use super::*;
    use std::time::Instant;

    #[test]
    fn queued_app_view_calls_and_replies_reauthorize_before_dispatch() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let tool = TestTool::new(
            r#"#!/bin/sh
set -eu
root=$1
while IFS= read -r request; do
  id=${request#*:}
  id=${id%%,*}
  case "$request" in
    *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
    *'"method":"browser.app.'*) : > "$root/view-sent"; printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
    *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
  esac
done
"#,
        );
        let store = BrowserControllerProcessStore::new(
            tool.path(),
            vec![tool.root.display().to_string()],
            Duration::from_secs(3),
        );
        store.acquire("room").unwrap();
        let live = Arc::new(AtomicBool::new(true));
        let authority = live.clone();
        let mut guarded = store.with_authorizer(Arc::new(move || {
            if authority.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err("revoked".into())
            }
        }));
        let probe = Arc::new(tokio::sync::Notify::new());
        guarded.observe_supervisor_lock_wait_for_test(probe.clone());
        for request in [
            BrowserAppViewRequest::Calls,
            BrowserAppViewRequest::Respond {
                target_id: "target".into(),
                call_id: "call".into(),
                result: None,
                error: None,
            },
        ] {
            live.store(true, Ordering::SeqCst);
            let ownership = store.ownership.as_ref().unwrap().lock().unwrap();
            let caller = guarded.clone();
            let pending = std::thread::spawn(move || caller.app_view("room", &request));
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    tokio::time::timeout(Duration::from_secs(2), probe.notified())
                        .await
                        .unwrap();
                });
            live.store(false, Ordering::SeqCst);
            drop(ownership);
            assert_eq!(pending.join().unwrap().unwrap_err(), "revoked");
            assert!(
                !tool.root.join("view-sent").exists(),
                "revoked queued work never reaches controller"
            );
        }
        store.release("room").unwrap();
    }

    #[test]
    fn app_view_idle_drain_does_not_hold_ownership_while_waiting() {
        use crate::runtime::browser_controller_app_view::BrowserAppViewRequest;
        let tool = TestTool::new(
            r#"#!/bin/sh
set -eu
root=$1
poll=
while IFS= read -r request; do
  id=${request#*:}
  id=${id%%,*}
  case "$request" in
    *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
    *'"method":"browser.app.calls"'*) poll=$id; : > "$root/drain-started" ;;
    *'"method":"browser.app.respond"'*)
      printf '{"id":%s,"ok":true,"result":{"delivered":true}}\n' "$id"
      printf '{"id":%s,"ok":true,"result":{"calls":[]}}\n' "$poll" ;;
    *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
  esac
done
"#,
        );
        let store = BrowserControllerProcessStore::new(
            tool.path(),
            vec![tool.root.display().to_string()],
            Duration::from_secs(3),
        );
        store.acquire("room").unwrap();
        assert!(store
            .app_view("other", &BrowserAppViewRequest::Calls)
            .is_err());
        let polling = store.clone();
        let drain =
            std::thread::spawn(move || polling.app_view("room", &BrowserAppViewRequest::Calls));
        let deadline = Instant::now() + Duration::from_secs(2);
        while !tool.root.join("drain-started").exists() {
            assert!(Instant::now() < deadline, "drain must reach the controller");
            std::thread::sleep(Duration::from_millis(5));
        }
        let reply = store
            .app_view(
                "room",
                &BrowserAppViewRequest::Respond {
                    target_id: "target".into(),
                    call_id: "call".into(),
                    result: Some(serde_json::json!(1)),
                    error: None,
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(reply["delivered"], true);
        assert!(drain.join().unwrap().is_ok());
        store.release("room").unwrap();
    }

    #[test]
    fn app_view_reply_burst_is_serialized_without_blocking_snapshots_or_drains() {
        let tool = TestTool::new(
            r#"#!/bin/sh
set -eu
root=$1
held=
while IFS= read -r request; do
  id=${request#*:}
  id=${id%%,*}
  case "$request" in
    *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
    *'"method":"browser.app.respond"'*)
      printf '%s\n' "$id" >> "$root/replies"
      if [ -e "$root/release" ]; then
        printf '{"id":%s,"ok":true,"result":{"delivered":true}}\n' "$id"
      else held=$id; fi ;;
    *'"method":"browser.snapshot"'*)
      printf '{"id":%s,"ok":true,"result":{"browser_generation":1,"target_id":"target","document_id":"doc","snapshot_revision":1,"accessibility_nodes":[],"dom_nodes":[]}}\n' "$id" ;;
    *'"method":"browser.app.calls"'*)
      printf '{"id":%s,"ok":true,"result":{"calls":[]}}\n' "$id"
      if [ -e "$root/release" ] && [ -n "$held" ]; then
        printf '{"id":%s,"ok":true,"result":{"delivered":true}}\n' "$held"; held=
      fi ;;
    *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
  esac
done
"#,
        );
        let store = BrowserControllerProcessStore::new(
            tool.path(),
            vec![tool.root.display().to_string()],
            Duration::from_secs(3),
        );
        store.acquire("room").unwrap();
        let replies: Vec<_> = (0..64)
            .map(|n| {
                let replying = store.clone();
                std::thread::spawn(move || {
                    replying.app_view(
                        "room",
                        &BrowserAppViewRequest::Respond {
                            target_id: "target".into(),
                            call_id: n.to_string(),
                            result: Some(serde_json::json!(1)),
                            error: None,
                        },
                    )
                })
            })
            .collect();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !tool.root.join("replies").exists() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            std::fs::read_to_string(tool.root.join("replies"))
                .unwrap()
                .lines()
                .count(),
            1,
            "queued replies must not occupy the shared response registry"
        );
        assert!(store
            .capture_browser_snapshot("room", "target", "doc")
            .unwrap()
            .is_some());
        std::fs::write(tool.root.join("release"), "").unwrap();
        assert!(store
            .app_view("room", &BrowserAppViewRequest::Calls)
            .is_ok());
        for reply in replies {
            assert!(reply.join().unwrap().is_ok());
        }
        store.release("room").unwrap();
    }
}
