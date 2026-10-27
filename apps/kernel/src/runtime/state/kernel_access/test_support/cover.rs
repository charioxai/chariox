//! A grant covers a named background process by identity; the cover follows the process
//! and the grant, never a recycled pid.
use super::*;
use std::process::Command;

/// A process the test process did not start (the kernel's own descendants are never
/// covered): `script` runs in a new session and `file` receives its pid.
fn detached(dir: &std::path::Path, name: &str, script: &str) -> u32 {
    let file = dir.join(name);
    Command::new("setsid")
        .args([
            "-f",
            "sh",
            "-c",
            &format!("echo $$ > {}; {script}", file.display()),
        ])
        .status()
        .unwrap();
    let started = Instant::now();
    loop {
        if let Some(pid) = std::fs::read_to_string(&file)
            .ok()
            .and_then(|pid| pid.trim().parse().ok())
        {
            return pid;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{name} did not start"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
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

// A process only has to be named once: the cover is recorded with the grant, so it is
// still there after the kernel restarts, and it ends, audited once, when the process does.
#[tokio::test]
async fn a_covered_process_survives_a_restart_and_ends_with_its_exit() {
    let root = crate::test_support::TestWorktree::new("cover-restart");
    let kernel = || {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
        config.user_config_path = root.path().join("config.toml");
        let app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
        crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            std::sync::Arc::new(tokio::sync::Mutex::new(app)),
            32,
        )
    };
    // The holder starts a background process, as an agent's shell tool does.
    let holder_pid = detached(
        root.path(),
        "holder",
        &format!(
            "sleep 600 & echo $! > {}; exec sleep 600",
            root.path().join("background").display()
        ),
    );
    let background = std::fs::read_to_string(root.path().join("background"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let holder = process::inspect(holder_pid).unwrap().0;
    let target = process::inspect(background).unwrap().0;
    let request = CoverKernelAccessProcessRequest {
        pid: background,
        start: None,
    };
    {
        let router = kernel();
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
    let router = kernel();
    let state = router.runtime_state();
    assert_eq!(state.restore_kernel_access_grants().unwrap(), 1);
    assert_eq!(
        state.owned.kernel_access.lock().unwrap().grants["cover"].covered,
        vec![target.clone()]
    );
    assert_eq!(state.access_grants_for(&target).len(), 1);

    kill_pid(background);
    assert!(state.list_kernel_access("local")[0].covered.is_empty());
    state.sweep_kernel_access();
    state.sweep_kernel_access();
    assert_eq!(
        outcomes(&state)
            .iter()
            .filter(|o| *o == "cover_ended")
            .count(),
        1
    );
    kill_pid(holder_pid);
}

// A pid that is reused after the covered process exits has another start time: the cover
// matches the process it named, not the number.
#[test]
fn a_cover_never_matches_a_process_that_reuses_the_pid() {
    let root = crate::test_support::TestWorktree::new("cover-reuse");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.state.path = Some(root.path().join("state.db").display().to_string());
    config.user_config_path = root.path().join("config.toml");
    let app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
        32,
    );
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
