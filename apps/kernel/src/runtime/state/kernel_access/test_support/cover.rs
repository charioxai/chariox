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
    assert_eq!(state.access_grants_for(&target).unwrap().len(), 1);

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
    assert_eq!(state.access_grants_for(&other).unwrap().len(), 1);
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
    assert!(state.access_grants_for(&other).unwrap().is_empty());
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
        let database = rusqlite::Connection::open(root.path().join("state.db")).unwrap();
        database
            .execute_batch(
                "CREATE TRIGGER fail_cover BEFORE INSERT ON durable_state_events
                 WHEN NEW.payload_json LIKE '%\"outcome\":\"covered\"%'
                 BEGIN SELECT RAISE(FAIL, 'injected cover failure'); END;",
            )
            .unwrap();
        // The background process already runs under the holder: the grant tries to cover
        // it as it lands, and the injected failure leaves it uncovered too.
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

// EA2-77: the command a covered process runs asks for the status of that process by pid, as
// `chariox access status --holder-pid <pid>` does from a covered background process. It is
// granted; a pid outside the caller's chain is still refused.
#[tokio::test]
async fn a_covered_process_named_by_its_own_command_is_granted() {
    let root = crate::test_support::TestWorktree::new("cover-own-status");
    let holder_pid = detached(
        root.path(),
        "holder",
        &format!(
            "sh -c 'sleep 600 & echo $! > {}; wait' & echo $! > {}; exec sleep 600",
            root.path().join("command").display(),
            root.path().join("background").display()
        ),
    );
    let wait_pid = |name: &str| {
        let file = root.path().join(name);
        let started = Instant::now();
        loop {
            if let Some(pid) = std::fs::read_to_string(&file)
                .ok()
                .and_then(|pid| pid.trim().parse::<u32>().ok())
                .filter(|pid| process::inspect(*pid).is_ok())
            {
                return pid;
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "{name} was not ready"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    let background = wait_pid("background");
    let command = wait_pid("command");
    let holder = process::inspect(holder_pid).unwrap().0;
    let target = process::inspect(command).unwrap().0;
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
    let named = state.kernel_access_status(&target, Some(background));
    let outside = state
        .kernel_access_status(&target, Some(99_999_999))
        .is_err();
    kill_pid(command);
    kill_pid(background);
    kill_pid(holder_pid);
    assert_eq!(named.unwrap().state, KernelAccessHolderState::Granted);
    assert!(outside, "a pid outside the caller's chain stays refused");
}

/// A holder shell, not started by the test process, with two `sleep`s already under it. When
/// `dir/go` appears it starts 18 more and lists their pids in `dir/late`.
fn holder_with_early_and_late_children(dir: &std::path::Path) -> ProcessIdentity {
    let script = format!(
        "echo $$ > {d}/holder; sleep 600 & sleep 600 & echo ready > {d}/ready; \
         while [ ! -f {d}/go ]; do sleep 0.05; done; \
         i=0; while [ $i -lt 18 ]; do sleep 600 & echo $! >> {d}/late; i=$((i+1)); done; wait",
        d = dir.display()
    );
    let status = Command::new("sh")
        .args(["-c", "sh -c \"$0\" >/dev/null 2>&1 &"])
        .arg(script)
        .status()
        .unwrap();
    assert!(status.success());
    let started = Instant::now();
    while !dir.join("ready").exists() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "holder not ready"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let pid = std::fs::read_to_string(dir.join("holder")).unwrap();
    process::inspect(pid.trim().parse().unwrap()).unwrap().0
}

fn late_pids(dir: &std::path::Path) -> Vec<u32> {
    let started = Instant::now();
    loop {
        let pids: Vec<u32> = std::fs::read_to_string(dir.join("late"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.parse().ok())
            .collect();
        if pids.len() == 18 {
            return pids;
        }
        assert!(started.elapsed() < Duration::from_secs(5), "late children");
        std::thread::sleep(Duration::from_millis(10));
    }
}

// The holder's tree at grant time is covered by identity and uses none of the 16 named
// covers; when those processes exit they free no extra named covers either.
#[test]
fn adopted_processes_use_no_named_cover_and_their_exit_frees_none() {
    let root = crate::test_support::TestWorktree::new("cover-adopted");
    let holder = holder_with_early_and_late_children(root.path());
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
    let adopted = access.grants["cover"].adopted.clone();
    assert!(adopted.len() >= 2, "{adopted:?}");
    assert_eq!(access.grants["cover"].covered, adopted);
    drop(access);
    for process in &adopted {
        kill_pid(process.pid);
    }
    std::fs::write(root.path().join("go"), "").unwrap();
    let late = late_pids(root.path());
    for pid in &late[..16] {
        state
            .cover_kernel_access_process(
                &holder,
                None,
                &CoverKernelAccessProcessRequest {
                    pid: *pid,
                    start: None,
                },
            )
            .unwrap();
    }
    let refused = state
        .cover_kernel_access_process(
            &holder,
            None,
            &CoverKernelAccessProcessRequest {
                pid: late[16],
                start: None,
            },
        )
        .unwrap_err();
    assert!(
        crate::runtime::kernel_access::Refusal::exact(&refused)
            == Some(crate::runtime::kernel_access::Refusal::CoverLimit),
        "{refused:?}"
    );
    for pid in late {
        kill_pid(pid);
    }
    kill_pid(holder.pid);
}

// After a restart a process named later is still named, not adopted.
#[test]
fn a_restart_keeps_adopted_and_named_covers_apart() {
    let root = crate::test_support::TestWorktree::new("cover-adopted-restart");
    let holder = holder_with_early_and_late_children(root.path());
    let adopted = {
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
        let adopted = access.grants["cover"].adopted.clone();
        drop(access);
        std::fs::write(root.path().join("go"), "").unwrap();
        let late = late_pids(root.path());
        state
            .cover_kernel_access_process(
                &holder,
                None,
                &CoverKernelAccessProcessRequest {
                    pid: late[0],
                    start: None,
                },
            )
            .unwrap();
        state.end_kernel_access_for_shutdown().unwrap();
        adopted
    };
    let router = kernel(&root);
    let state = router.runtime_state();
    assert_eq!(state.restore_kernel_access_grants().unwrap(), 1);
    let access = state.owned.kernel_access.lock().unwrap();
    let grant = &access.grants["cover"];
    // A transient `sleep` of the holder's wait loop may have ended since.
    let alive = adopted.iter().filter(|process| process.alive()).count();
    assert_eq!(grant.adopted.len(), alive, "{:?}", grant.adopted);
    assert!(grant.covered.len() > grant.adopted.len());
    drop(access);
    for pid in late_pids(root.path()) {
        kill_pid(pid);
    }
    kill_pid(holder.pid);
}
