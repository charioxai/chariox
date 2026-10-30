//! A host under memory or descriptor pressure can fail to read a live process. That
//! proves nothing about the process: only a confirmed exit ends its grant, and every
//! end tells the process that asks why.
use super::*;
use crate::runtime::kernel_access::process::fault;
use std::process::Command;

/// Out of memory, out of system files, out of descriptors, an I/O error.
const TRANSIENT: [i32; 4] = [libc::ENOMEM, libc::ENFILE, libc::EMFILE, libc::EIO];

fn state() -> KernelRuntimeState {
    let app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
        32,
    )
    .runtime_state()
}

/// A `sleep` the test process did not start (the kernel's own descendants never hold
/// grants): the launcher exits at once and the script, orphaned, ends as `sleep`.
fn sleeper(dir: &std::path::Path, name: &str) -> ProcessIdentity {
    let file = dir.join(name);
    let status = Command::new("sh")
        .args(["-c", "sh -c \"$0\" >/dev/null 2>&1 &"])
        .arg(format!("echo $$ > {}; exec sleep 600", file.display()))
        .status()
        .unwrap();
    assert!(status.success());
    let started = Instant::now();
    loop {
        if let Some(identity) = std::fs::read_to_string(&file)
            .ok()
            .and_then(|pid| pid.trim().parse().ok())
            .and_then(|pid| process::inspect(pid).ok())
            .map(|(identity, _)| identity)
            .filter(|identity| identity.executable.ends_with("/sleep"))
        {
            return identity;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{name} not ready"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn stop(identity: &ProcessIdentity) {
    assert!(identity.pid > 1);
    unsafe { libc::kill(identity.pid as i32, libc::SIGKILL) };
    let started = Instant::now();
    while identity.alive() {
        assert!(started.elapsed() < Duration::from_secs(5), "did not exit");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn grant(
    id: &str,
    term: KernelAccessTerm,
    clock: Option<GrantClock>,
    holder: &ProcessIdentity,
    covered: Vec<ProcessIdentity>,
) -> Grant {
    Grant {
        covered,
        summary: KernelAccessGrant {
            covered: Vec::new(),
            grant_id: id.into(),
            owner_user_id: "local".into(),
            holder_pid: holder.pid,
            holder_executable: holder.executable.clone(),
            holder_harness: None,
            term,
            expires_at_ms: None,
        },
        holder: holder.clone(),
        clock,
    }
}

fn insert(state: &KernelRuntimeState, grant: Grant) {
    state
        .owned
        .kernel_access
        .lock()
        .unwrap()
        .grants
        .insert(grant.summary.grant_id.clone(), grant);
}

fn live(state: &KernelRuntimeState, id: &str) -> Option<Grant> {
    state
        .owned
        .kernel_access
        .lock()
        .unwrap()
        .grants
        .get(id)
        .cloned()
}

fn cover_request(pid: u32) -> CoverKernelAccessProcessRequest {
    CoverKernelAccessProcessRequest { pid, start: None }
}

#[tokio::test]
async fn an_until_exit_grant_survives_a_failure_to_read_its_live_holder() {
    let state = state();
    let root = crate::test_support::TestWorktree::new("pressure-survives");
    let holder = sleeper(root.path(), "holder");
    let covered = sleeper(root.path(), "covered");
    insert(
        &state,
        grant(
            "until-exit",
            KernelAccessTerm::UntilHolderExits,
            None,
            &holder,
            vec![covered.clone()],
        ),
    );
    for errno in TRANSIENT {
        let _holder = fault::inject(holder.pid, errno);
        let _covered = fault::inject(covered.pid, errno);
        state.sweep_kernel_access_at(Instant::now());
        let kept = live(&state, "until-exit").unwrap_or_else(|| panic!("ended by errno {errno}"));
        assert_eq!(kept.covered, [covered.clone()], "errno {errno}");
        assert!(
            state.owned.kernel_access.lock().unwrap().ended.is_empty(),
            "errno {errno}"
        );
    }
    state.sweep_kernel_access_at(Instant::now());
    assert!(live(&state, "until-exit").is_some());
    stop(&covered);
    state.sweep_kernel_access_at(Instant::now());
    let kept = live(&state, "until-exit").expect("the holder still runs");
    assert!(
        kept.covered.is_empty(),
        "a covered process that exited is released"
    );
    stop(&holder);
    state.sweep_kernel_access_at(Instant::now());
    assert!(live(&state, "until-exit").is_none(), "the holder exited");
}

#[tokio::test]
async fn a_failure_to_read_a_caller_refuses_that_request_and_keeps_the_grant() {
    let state = state();
    let root = crate::test_support::TestWorktree::new("pressure-refuses");
    let holder = sleeper(root.path(), "holder");
    let other = sleeper(root.path(), "other");
    insert(
        &state,
        grant(
            "until-exit",
            KernelAccessTerm::UntilHolderExits,
            None,
            &holder,
            Vec::new(),
        ),
    );
    let refused = {
        let _fault = fault::inject(holder.pid, libc::ENOMEM);
        state
            .cover_kernel_access_process(&holder, None, &cover_request(other.pid))
            .unwrap_err()
            .to_string()
    };
    assert!(
        refused.contains("could not confirm"),
        "a transient failure is retryable, not 'not covered' or 'ended': {refused}"
    );
    assert!(live(&state, "until-exit").is_some());
    // The same request goes on as any other once the host answers again.
    let retried = state
        .cover_kernel_access_process(&holder, None, &cover_request(other.pid))
        .unwrap_err()
        .to_string();
    assert!(
        retried.contains("Only a process your access already reaches"),
        "{retried}"
    );
    stop(&other);
    stop(&holder);
}

#[tokio::test]
async fn a_caller_whose_grant_ended_is_told_why() {
    let cases = [
        ("expiry", "time ran out"),
        ("explicit_revoke", "was revoked"),
        ("passkey_rotation", "passkey changed"),
    ];
    for (reason, said) in cases {
        let state = state();
        let root = crate::test_support::TestWorktree::new("pressure-told-why");
        let holder = sleeper(root.path(), "holder");
        let other = sleeper(root.path(), "other");
        insert(
            &state,
            grant(
                "ended",
                KernelAccessTerm::Minutes(30),
                Some(GrantClock::new(30, 5)),
                &holder,
                Vec::new(),
            ),
        );
        if reason == "expiry" {
            state.sweep_kernel_access_at(Instant::now() + Duration::from_secs(3 * 3600));
        } else {
            state
                .revoke_kernel_access(None, Some("ended"), reason)
                .unwrap();
        }
        assert!(live(&state, "ended").is_none());
        let refused = state
            .cover_kernel_access_process(&holder, None, &cover_request(other.pid))
            .unwrap_err()
            .to_string();
        assert!(refused.contains(said), "{reason}: {refused}");
        assert!(
            refused.contains("Ask for new access"),
            "{reason}: {refused}"
        );
        stop(&other);
        stop(&holder);
    }
}

/// Holds every descriptor the process may open, as a busy host does, until dropped.
struct Exhausted {
    saved: libc::rlimit,
    held: Vec<std::fs::File>,
}

impl Exhausted {
    fn new() -> Self {
        let mut saved: libc::rlimit = unsafe { std::mem::zeroed() };
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut saved) },
            0
        );
        let open = std::fs::read_dir("/proc/self/fd").unwrap().count() as libc::rlim_t;
        let lowered = libc::rlimit {
            rlim_cur: open + 8,
            rlim_max: saved.rlim_max,
        };
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &lowered) }, 0);
        let mut held = Vec::new();
        loop {
            match std::fs::File::open("/dev/null") {
                Ok(file) => held.push(file),
                Err(error) if error.raw_os_error() == Some(libc::EMFILE) => break,
                Err(error) => panic!("{error}"),
            }
        }
        Self { saved, held }
    }
}

impl Drop for Exhausted {
    fn drop(&mut self) {
        self.held.clear();
        unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &self.saved) };
    }
}

// The same failure with a real OS error: a process that cannot open `/proc/<pid>/stat`
// tells nothing about the holder.
#[tokio::test]
async fn a_host_out_of_descriptors_does_not_end_a_live_holders_grant() {
    let state = state();
    let root = crate::test_support::TestWorktree::new("pressure-descriptors");
    let holder = sleeper(root.path(), "holder");
    insert(
        &state,
        grant(
            "until-exit",
            KernelAccessTerm::UntilHolderExits,
            None,
            &holder,
            Vec::new(),
        ),
    );
    {
        let _exhausted = Exhausted::new();
        state.sweep_kernel_access_at(Instant::now());
    }
    assert!(live(&state, "until-exit").is_some());
    stop(&holder);
    state.sweep_kernel_access_at(Instant::now());
    assert!(live(&state, "until-exit").is_none());
}

fn ended_events(state: &KernelRuntimeState, id: &str) -> Vec<(String, String)> {
    state
        .owned
        .durable_state_store
        .load_events_by_kind("kernel_access.grant")
        .unwrap()
        .into_iter()
        .filter(|event| event.payload["grant"]["grant_id"] == id)
        .filter_map(|event| {
            let outcome = event.payload["outcome"].as_str()?;
            let reason = event.payload["reason"].as_str()?;
            matches!(outcome, "revoked" | "expired")
                .then(|| (outcome.to_owned(), reason.to_owned()))
        })
        .collect()
}

#[tokio::test]
async fn every_grant_end_is_logged_and_audited_with_its_reason() {
    let capture = crate::logging::capture::start();
    let state = state();
    let root = crate::test_support::TestWorktree::new("pressure-logged");
    let holder = sleeper(root.path(), "holder");
    let timed = |id: &str| {
        grant(
            id,
            KernelAccessTerm::Minutes(30),
            Some(GrantClock::new(30, 5)),
            &holder,
            Vec::new(),
        )
    };
    insert(&state, timed("end-expiry"));
    insert(&state, timed("end-revoked"));
    insert(&state, timed("end-passkey"));
    insert(
        &state,
        grant(
            "end-exit",
            KernelAccessTerm::UntilHolderExits,
            None,
            &holder,
            Vec::new(),
        ),
    );
    state
        .revoke_kernel_access(None, Some("end-revoked"), "revoked_by_owner")
        .unwrap();
    state
        .revoke_kernel_access(None, Some("end-passkey"), "passkey_rotation")
        .unwrap();
    state.sweep_kernel_access_at(Instant::now() + Duration::from_secs(3 * 3600));
    stop(&holder);
    state.sweep_kernel_access_at(Instant::now());
    let records = capture.records();
    for (id, outcome, reason) in [
        ("end-expiry", "expired", "expiry"),
        ("end-revoked", "revoked", "revoked_by_owner"),
        ("end-passkey", "revoked", "passkey_rotation"),
        ("end-exit", "revoked", "process_exit"),
    ] {
        assert!(
            ended_events(&state, id).contains(&(outcome.to_owned(), reason.to_owned())),
            "{id} is audited as {outcome}/{reason}"
        );
        assert!(
            records
                .iter()
                .any(|record| record.contains("kernel access grant ended")
                    && record.contains(id)
                    && record.contains(reason)),
            "{id} is logged with {reason}"
        );
    }
}
