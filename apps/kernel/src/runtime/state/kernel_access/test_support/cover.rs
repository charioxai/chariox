//! A grant covers a named background process by identity; the cover follows the process
//! and the grant, never a recycled pid.
use super::*;
use std::process::Command;

/// The executable every script ends as (`sleep`), asked of a real one: `spawn` returns
/// once the exec happened, whereas a shell that has not exec'd yet shows the shell's.
fn sleep_executable() -> &'static str {
    static EXECUTABLE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    EXECUTABLE.get_or_init(|| {
        let mut sleep = Command::new("sleep").arg("600").spawn().unwrap();
        let executable = process::inspect(sleep.id()).unwrap().0.executable;
        sleep.kill().unwrap();
        sleep.wait().unwrap();
        executable
    })
}

/// The pid `file` names once the script is done with it: the file exists and the process
/// is already `sleep`.
fn ready_pid(file: &std::path::Path) -> u32 {
    let started = Instant::now();
    loop {
        if let Some(pid) = std::fs::read_to_string(file)
            .ok()
            .and_then(|pid| pid.trim().parse().ok())
            .filter(|pid| {
                process::inspect(*pid).is_ok_and(|(p, _)| p.executable == sleep_executable())
            })
        {
            return pid;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{} was not ready",
            file.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A process the test process did not start (the kernel's own descendants are never
/// covered): the launcher exits at once, so `script` is orphaned, and `file` receives
/// its pid. `script` ends in `exec sleep`; the pid is returned once it has.
fn detached(dir: &std::path::Path, name: &str, script: &str) -> u32 {
    let file = dir.join(name);
    let status = Command::new("sh")
        .args(["-c", "sh -c \"$0\" >/dev/null 2>&1 &"])
        .arg(format!("echo $$ > {}; {script}", file.display()))
        .status()
        .unwrap();
    assert!(status.success());
    ready_pid(&file)
}

/// The `sleep` the holder script started and named in `background`.
fn background_pid(dir: &std::path::Path) -> u32 {
    ready_pid(&dir.join("background"))
}

fn kill_pid(pid: u32) {
    assert!(pid > 1);
    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    let started = Instant::now();
    while process::inspect(pid).is_ok() && started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn summary(holder: &ProcessIdentity) -> KernelAccessGrant {
    KernelAccessGrant {
        covered: Vec::new(),
        grant_id: "cover".into(),
        owner_user_id: "local".into(),
        holder_pid: holder.pid,
        holder_executable: holder.executable.clone(),
        holder_harness: None,
        term: KernelAccessTerm::UntilHolderExits,
        expires_at_ms: None,
    }
}

// The helper returns a process that is already its final program, with every pid file the
// script writes in place, however late the script gets there.
#[test]
fn a_late_script_is_complete_when_the_helper_returns() {
    let root = crate::test_support::TestWorktree::new("cover-late");
    let holder_pid = detached(
        root.path(),
        "holder",
        &format!(
            "sleep 0.5; sleep 600 & echo $! > {}; exec sleep 600",
            root.path().join("background").display()
        ),
    );
    let background = background_pid(root.path());
    for pid in [holder_pid, background] {
        assert_eq!(
            process::inspect(pid).unwrap().0.executable,
            sleep_executable()
        );
    }
    kill_pid(background);
    kill_pid(holder_pid);
}

fn kernel(root: &crate::test_support::TestWorktree) -> crate::runtime::router::CommandRouter {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
    config.user_config_path = root.path().join("config.toml");
    let app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
        32,
    )
}

fn outcomes(state: &KernelRuntimeState) -> Vec<String> {
    state
        .owned
        .durable_state_store
        .load_events_by_kind("kernel_access.grant")
        .unwrap()
        .into_iter()
        .filter_map(|event| event.payload["outcome"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn every_grant_end_audits_each_remaining_covered_process_once() {
    for reason in [
        "process_exit",
        "revoked_by_owner",
        "expiry",
        "kernel_shutdown",
    ] {
        let root = crate::test_support::TestWorktree::new("cover-grant-end");
        let holder_pid = detached(
            root.path(),
            "holder",
            &format!(
                "sleep 600 & echo $! > {}; exec sleep 600",
                root.path().join("background").display()
            ),
        );
        let background = background_pid(root.path());
        let holder = process::inspect(holder_pid).unwrap().0;
        let target = process::inspect(background).unwrap().0;
        let router = kernel(&root);
        let state = router.runtime_state();
        let mut grant = summary(&holder);
        let clock = (reason == "expiry" || reason == "kernel_shutdown").then(|| {
            grant.term = KernelAccessTerm::Minutes(30);
            GrantClock::new(30, 5)
        });
        state
            .admit_grant(
                &mut state.owned.kernel_access.lock().unwrap(),
                "cover".into(),
                grant,
                holder.clone(),
                clock,
            )
            .unwrap();
        state
            .cover_kernel_access_process(
                &holder,
                None,
                &CoverKernelAccessProcessRequest {
                    pid: background,
                    start: None,
                },
            )
            .unwrap();
        match reason {
            "process_exit" => {
                kill_pid(holder_pid);
                state.sweep_kernel_access();
            }
            "expiry" => state.sweep_kernel_access_at(Instant::now() + Duration::from_secs(3600)),
            "kernel_shutdown" => {
                state.end_kernel_access_for_shutdown().unwrap();
            }
            _ => {
                state
                    .revoke_kernel_access(None, Some("cover"), reason)
                    .unwrap();
            }
        }
        state.sweep_kernel_access();
        let events = state
            .owned
            .durable_state_store
            .load_events_by_kind("kernel_access.grant")
            .unwrap();
        let ends: Vec<_> = events
            .iter()
            .filter(|event| event.payload["outcome"] == "cover_ended")
            .collect();
        kill_pid(background);
        if reason != "process_exit" {
            kill_pid(holder_pid);
        }
        assert_eq!(
            ends.len(),
            1,
            "{reason}: every covered process needs its own end"
        );
        assert_eq!(ends[0].payload["reason"], reason);
        assert_eq!(
            serde_json::from_value::<ProcessIdentity>(ends[0].payload["process"].clone()).unwrap(),
            target
        );
    }
}

#[tokio::test]
async fn a_failed_cover_end_is_finished_after_restart_without_restoring_authority() {
    let root = crate::test_support::TestWorktree::new("cover-end-retry");
    let holder_pid = detached(
        root.path(),
        "holder",
        &format!(
            "sleep 600 & echo $! > {}; exec sleep 600",
            root.path().join("background").display()
        ),
    );
    let background = background_pid(root.path());
    let holder = process::inspect(holder_pid).unwrap().0;
    let database = rusqlite::Connection::open(root.path().join("state.db")).unwrap();
    {
        let router = kernel(&root);
        let state = router.runtime_state();
        state
            .admit_grant(
                &mut state.owned.kernel_access.lock().unwrap(),
                "cover".into(),
                summary(&holder),
                holder.clone(),
                None,
            )
            .unwrap();
        state
            .cover_kernel_access_process(
                &holder,
                None,
                &CoverKernelAccessProcessRequest {
                    pid: background,
                    start: None,
                },
            )
            .unwrap();
        database
            .execute_batch(
                "CREATE TRIGGER fail_cover_end BEFORE INSERT ON durable_state_events
             WHEN NEW.payload_json LIKE '%\"outcome\":\"cover_ended\"%'
             BEGIN SELECT RAISE(FAIL, 'injected cover end failure'); END;",
            )
            .unwrap();
        assert!(state
            .revoke_kernel_access(None, Some("cover"), "revoked_by_owner")
            .is_err());
        assert!(state.owned.kernel_access.lock().unwrap().grants.is_empty());
        assert!(state.outside_ended_response("cover").unwrap().is_some());
        let outcomes = outcomes(&state);
        assert_eq!(outcomes.iter().filter(|o| *o == "revoked").count(), 1);
        assert!(!outcomes.iter().any(|o| o == "cover_ended"));
    }
    database
        .execute_batch("DROP TRIGGER fail_cover_end;")
        .unwrap();
    let router = kernel(&root);
    let state = router.runtime_state();
    assert_eq!(state.restore_kernel_access_grants().unwrap(), 0);
    assert!(state.owned.kernel_access.lock().unwrap().grants.is_empty());
    state.sweep_kernel_access();
    let outcomes = outcomes(&state);
    kill_pid(background);
    kill_pid(holder_pid);
    for outcome in ["revoked", "cover_ended"] {
        assert_eq!(
            outcomes.iter().filter(|o| *o == outcome).count(),
            1,
            "{outcomes:?}"
        );
    }
}

// A process only has to be named once: the cover is recorded with the grant, so it is
// still there after the kernel restarts, and it ends, audited once, when the process does.
#[tokio::test]
async fn a_covered_process_survives_a_restart_and_ends_with_its_exit() {
    let root = crate::test_support::TestWorktree::new("cover-restart");
    // The holder starts a background process, as an agent's shell tool does.
    let holder_pid = detached(
        root.path(),
        "holder",
        &format!(
            "sleep 600 & echo $! > {}; exec sleep 600",
            root.path().join("background").display()
        ),
    );
    let background = background_pid(root.path());
    let holder = process::inspect(holder_pid).unwrap().0;
    let target = process::inspect(background).unwrap().0;
    let request = CoverKernelAccessProcessRequest {
        pid: background,
        start: None,
    };
    {
        let router = kernel(&root);
        let state = router.runtime_state();
        let mut access = state.owned.kernel_access.lock().unwrap();
        state
            .admit_grant(
                &mut access,
                "cover".into(),
                summary(&holder),
                holder.clone(),
                None,
            )
            .unwrap();
        drop(access);
        let (grant, covered) = state
            .cover_kernel_access_process(&holder, None, &request)
            .unwrap();
        assert_eq!((grant.as_str(), covered.pid), ("cover", background));
        assert_eq!(covered.start, target.start);
        assert_eq!(state.list_kernel_access("local")[0].covered, vec![covered]);
        state.end_kernel_access_for_shutdown().unwrap();
    }
    let router = kernel(&root);
    let state = router.runtime_state();
    assert_eq!(state.restore_kernel_access_grants().unwrap(), 1);
    assert_eq!(
        state.owned.kernel_access.lock().unwrap().grants["cover"].covered,
        vec![target.clone()]
    );
    assert_eq!(state.access_grants_for(&target).len(), 1);

    // The kernel's own descendants are never covered, even when the grant reaches them.
    let mut kernel_started = std::process::Command::new("sleep")
        .arg("600")
        .spawn()
        .unwrap();
    let refused = state
        .cover_kernel_access_process(
            &holder,
            None,
            &CoverKernelAccessProcessRequest {
                pid: kernel_started.id(),
                start: None,
            },
        )
        .unwrap_err();
    assert_eq!(Refusal::exact(&refused), Some(Refusal::CoverKernelProcess));
    kernel_started.kill().unwrap();
    kernel_started.wait().unwrap();

    kill_pid(background);
    assert!(state.list_kernel_access("local")[0].covered.is_empty());
    state.sweep_kernel_access();
    state.sweep_kernel_access();
    let outcomes = outcomes(&state);
    for outcome in ["covered", "cover_ended"] {
        assert_eq!(
            outcomes.iter().filter(|o| *o == outcome).count(),
            1,
            "{outcomes:?}"
        );
    }
    kill_pid(holder_pid);
}

// A pid that is reused after the covered process exits has another start time: the cover
// matches the process it named, not the number.
#[test]
fn a_cover_never_matches_a_process_that_reuses_the_pid() {
    let root = crate::test_support::TestWorktree::new("cover-reuse");
    let router = kernel(&root);
    let state = router.runtime_state();
    let holder_pid = detached(root.path(), "holder", "exec sleep 600");
    let other_pid = detached(root.path(), "other", "exec sleep 600");
    let holder = process::inspect(holder_pid).unwrap().0;
    let other = process::inspect(other_pid).unwrap().0;
    let mut access = state.owned.kernel_access.lock().unwrap();
    state
        .admit_grant(&mut access, "cover".into(), summary(&holder), holder, None)
        .unwrap();
    // `other` is no descendant of the holder: only the cover reaches it.
    access.grants.get_mut("cover").unwrap().covered = vec![other.clone()];
    drop(access);
    assert_eq!(state.access_grants_for(&other).len(), 1);
    assert!(state.access_grant_live("cover", &other));

    let mut reused = other.clone();
    reused.start += 1;
    state
        .owned
        .kernel_access
        .lock()
        .unwrap()
        .grants
        .get_mut("cover")
        .unwrap()
        .covered = vec![reused];
    assert!(state.access_grants_for(&other).is_empty());
    assert!(!state.access_grant_live("cover", &other));
    kill_pid(holder_pid);
    kill_pid(other_pid);
}

// A cover that could not be recorded is not live: the request fails, a retry records it,
// and the cover is still there after a restart.
#[tokio::test]
async fn a_cover_that_fails_to_persist_is_not_live_and_a_retry_records_it() {
    let root = crate::test_support::TestWorktree::new("cover-persist");
    let holder_pid = detached(
        root.path(),
        "holder",
        &format!(
            "sleep 600 & echo $! > {}; exec sleep 600",
            root.path().join("background").display()
        ),
    );
    let background = background_pid(root.path());
    let holder = process::inspect(holder_pid).unwrap().0;
    let target = process::inspect(background).unwrap().0;
    let request = CoverKernelAccessProcessRequest {
        pid: background,
        start: None,
    };
    {
        let router = kernel(&root);
        let state = router.runtime_state();
        let mut access = state.owned.kernel_access.lock().unwrap();
        state
            .admit_grant(
                &mut access,
                "cover".into(),
                summary(&holder),
                holder.clone(),
                None,
            )
            .unwrap();
        drop(access);
        let database = rusqlite::Connection::open(root.path().join("state.db")).unwrap();
        database
            .execute_batch(
                "CREATE TRIGGER fail_cover BEFORE INSERT ON durable_state_events
                 WHEN NEW.payload_json LIKE '%\"outcome\":\"covered\"%'
                 BEGIN SELECT RAISE(FAIL, 'injected cover failure'); END;",
            )
            .unwrap();
        state
            .cover_kernel_access_process(&holder, None, &request)
            .unwrap_err();
        assert!(state.list_kernel_access("local")[0].covered.is_empty());

        database.execute_batch("DROP TRIGGER fail_cover;").unwrap();
        state
            .cover_kernel_access_process(&holder, None, &request)
            .unwrap();
        assert_eq!(
            outcomes(&state).iter().filter(|o| *o == "covered").count(),
            1
        );
        state.end_kernel_access_for_shutdown().unwrap();
    }
    let router = kernel(&root);
    let state = router.runtime_state();
    assert_eq!(state.restore_kernel_access_grants().unwrap(), 1);
    assert_eq!(
        state.owned.kernel_access.lock().unwrap().grants["cover"].covered,
        vec![target]
    );
    kill_pid(background);
    kill_pid(holder_pid);
}
