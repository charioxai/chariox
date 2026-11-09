//! A holder the owner listed under `kernel_access.standing_holders` is admitted for
//! `until-exit` access without a popup; every other request waits for the owner.
use super::*;
use std::process::Command;

/// A holder shell and a requester under it, both re-parented to init, so the requester sits
/// in the holder's tree but not in the kernel's (the test process).
struct OrphanHolder {
    dir: std::path::PathBuf,
    holder: u32,
    requester: u32,
}

impl OrphanHolder {
    fn start(root: &crate::test_support::TestWorktree, name: &str) -> Self {
        let dir = root.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let status = Command::new("sh")
            .args([
                "-c",
                r#"( sh -c 'echo $$ >"$0/holder"; sleep 600 & echo $! >"$0/requester"; wait' "$0" & )"#,
            ])
            .arg(&dir)
            .status()
            .unwrap();
        assert!(status.success());
        let read = |file: &str| {
            let started = Instant::now();
            loop {
                if let Some(pid) = std::fs::read_to_string(dir.join(file))
                    .ok()
                    .and_then(|text| text.trim().parse().ok())
                {
                    break pid;
                }
                assert!(started.elapsed() < Duration::from_secs(5), "{file} missing");
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        Self {
            holder: read("holder"),
            requester: read("requester"),
            dir,
        }
    }

    fn executable(&self) -> String {
        process::inspect(self.holder).unwrap().0.executable
    }

    /// The request is answered without the owner, or the test fails instead of waiting.
    async fn admitted(
        &self,
        state: &KernelRuntimeState,
        term: KernelAccessTerm,
    ) -> KernelAccessGrant {
        tokio::time::timeout(Duration::from_secs(5), self.request(state, term))
            .await
            .expect("the request is waiting for an owner popup")
            .unwrap()
    }

    async fn request(
        &self,
        state: &KernelRuntimeState,
        term: KernelAccessTerm,
    ) -> Result<KernelAccessGrant, DaemonError> {
        state
            .request_kernel_access(
                process::inspect(self.requester).unwrap().0,
                RequestKernelAccessRequest {
                    holder_pid: Some(self.holder),
                    no_wait: false,
                    term: Some(term),
                },
            )
            .await
    }
}

impl Drop for OrphanHolder {
    fn drop(&mut self) {
        for pid in [self.requester, self.holder] {
            if pid > 1 {
                unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn list_standing(root: &crate::test_support::TestWorktree, harness: &str, executable: &str) {
    std::fs::write(
        root.path().join("config.toml"),
        format!(
            "[[kernel_access.standing_holders]]\nharness = \"{harness}\"\nexecutable = \"{executable}\"\n"
        ),
    )
    .unwrap();
}

fn grant_events(state: &KernelRuntimeState) -> Vec<(String, Option<String>)> {
    state
        .owned
        .durable_state_store
        .load_events_by_kind("kernel_access.grant")
        .unwrap()
        .into_iter()
        .map(|event| {
            (
                event.payload["outcome"].as_str().unwrap().to_owned(),
                event.payload["reason"].as_str().map(str::to_owned),
            )
        })
        .collect()
}

// A listed holder is admitted at once and audited as a standing grant; revoking it ends it
// like any grant, and the next request of the same holder is admitted again.
#[tokio::test]
async fn a_listed_holder_is_admitted_without_a_popup_and_revoking_does_not_stop_the_next() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("access-standing");
    let holder = OrphanHolder::start(&root, "holder");
    std::env::set_var("CHARIOX_CLAUDE_BIN", holder.executable());
    list_standing(&root, "claude", &holder.executable());
    let router = reopen_kernel(&root);
    let state = router.runtime_state();
    let grant = holder
        .admitted(&state, KernelAccessTerm::UntilHolderExits)
        .await;
    assert_eq!(grant.holder_pid, holder.holder);
    assert_eq!(grant.expires_at_ms, None);
    assert_eq!(state.list_kernel_access(&grant.owner_user_id).len(), 1);
    assert_eq!(
        grant_events(&state),
        [("granted".to_owned(), Some("standing_policy".to_owned()))]
    );
    assert_eq!(
        state
            .revoke_kernel_access(
                Some(&grant.owner_user_id),
                Some(&grant.grant_id),
                "revoked_by_owner"
            )
            .unwrap(),
        1
    );
    assert!(state.list_kernel_access(&grant.owner_user_id).is_empty());
    let again = holder
        .admitted(&state, KernelAccessTerm::UntilHolderExits)
        .await;
    assert_ne!(again.grant_id, grant.grant_id);
}

// The grant is an ordinary until-exit grant: a kernel restart brings it back while the holder
// lives.
#[tokio::test]
async fn a_standing_grant_survives_a_restart_while_its_holder_lives() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("access-standing-restart");
    let holder = OrphanHolder::start(&root, "holder");
    std::env::set_var("CHARIOX_CLAUDE_BIN", holder.executable());
    list_standing(&root, "claude", &holder.executable());
    let id = {
        let router = reopen_kernel(&root);
        let state = router.runtime_state();
        let grant = holder
            .admitted(&state, KernelAccessTerm::UntilHolderExits)
            .await;
        state.end_kernel_access_for_shutdown().unwrap();
        grant.grant_id
    };
    let router = reopen_kernel(&root);
    let state = router.runtime_state();
    assert_eq!(state.restore_kernel_access_grants().unwrap(), 1);
    assert!(state
        .owned
        .kernel_access
        .lock()
        .unwrap()
        .grants
        .contains_key(&id));
}

// Only an until-exit request of a listed harness and executable skips the popup.
#[tokio::test]
async fn a_request_that_does_not_match_the_list_waits_for_the_owner() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("access-standing-miss");
    let holder = OrphanHolder::start(&root, "holder");
    std::env::set_var("CHARIOX_CLAUDE_BIN", holder.executable());
    let router = reopen_kernel(&root);
    let state = router.runtime_state();
    // Each case leaves its request pending on the owner's popup: the timeout is the proof.
    for (config, term) in [
        (None, KernelAccessTerm::UntilHolderExits),
        (
            Some(("claude", holder.executable())),
            KernelAccessTerm::Minutes(30),
        ),
        (
            Some(("claude", "/not/the/holder".to_owned())),
            KernelAccessTerm::UntilHolderExits,
        ),
        (
            Some(("codex", holder.executable())),
            KernelAccessTerm::UntilHolderExits,
        ),
    ] {
        if let Some((harness, executable)) = &config {
            list_standing(&root, harness, executable);
        }
        let waited =
            tokio::time::timeout(Duration::from_millis(600), holder.request(&state, term)).await;
        assert!(waited.is_err(), "{config:?} {term:?} was admitted");
        state
            .revoke_kernel_access(None, None, "revoked_by_owner")
            .unwrap();
    }
    std::fs::write(
        root.path().join("config.toml"),
        format!(
            "[[kernel_access.standing_holders]]\nharness = \"claude\"\n\n[[kernel_access.standing_holders]]\nharness = \"claude\"\nexecutable = \"{}\"\n",
            holder.executable()
        ),
    )
    .unwrap();
    let grant = holder
        .admitted(&state, KernelAccessTerm::UntilHolderExits)
        .await;
    assert_eq!(
        grant.holder_pid, holder.holder,
        "a malformed entry hid the valid one"
    );
}

// EXT-24: status answers for one holder identity in the caller's own chain, never with the
// caller's grant for an unrelated or missing pid.
#[tokio::test]
async fn status_reports_the_named_holder_and_refuses_pids_outside_the_callers_chain() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("access-status-identity");
    let holder = OrphanHolder::start(&root, "holder");
    let unrelated = OrphanHolder::start(&root, "unrelated");
    std::env::set_var("CHARIOX_CLAUDE_BIN", holder.executable());
    list_standing(&root, "claude", &holder.executable());
    let router = reopen_kernel(&root);
    let state = router.runtime_state();
    let grant = holder
        .admitted(&state, KernelAccessTerm::UntilHolderExits)
        .await;
    let requester = process::inspect(holder.requester).unwrap().0;
    let status = state
        .kernel_access_status(&requester, Some(holder.holder))
        .unwrap();
    assert_eq!(status.state, KernelAccessHolderState::Granted);
    assert_eq!(status.grant.unwrap().grant_id, grant.grant_id);
    for pid in [unrelated.holder, unrelated.requester, 99_999_999] {
        let refused = state
            .kernel_access_status(&requester, Some(pid))
            .unwrap_err();
        assert_eq!(Refusal::exact(&refused), Some(Refusal::HolderOutside));
    }
    let outsider = process::inspect(unrelated.requester).unwrap().0;
    assert!(state
        .kernel_access_status(&outsider, Some(holder.holder))
        .is_err());
    assert_eq!(
        state
            .kernel_access_status(&outsider, Some(unrelated.holder))
            .unwrap()
            .state,
        KernelAccessHolderState::NotRequested
    );
}

// A program whose launcher exited has no grandparent for the kernel to pick as holder. The
// refusal must say what to do next, in words that survive the external boundary.
#[tokio::test]
async fn a_requester_without_a_grandparent_is_told_how_to_name_its_holder() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("access-holder-unavailable");
    let orphan = OrphanHolder::start(&root, "orphan");
    let router = reopen_kernel(&root);
    let state = router.runtime_state();
    let requester = process::inspect(orphan.requester).unwrap().0;
    let refused = state
        .request_kernel_access(
            requester,
            RequestKernelAccessRequest {
                holder_pid: None,
                no_wait: true,
                term: Some(KernelAccessTerm::UntilHolderExits),
            },
        )
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("--holder-pid"),
        "the refusal names the way out, got: {refused}"
    );
    assert_eq!(Refusal::exact(&refused), Some(Refusal::HolderUnavailable));
}
