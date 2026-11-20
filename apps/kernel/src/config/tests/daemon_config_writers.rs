use super::*;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const SIBLING_REMOVE: &str = "CHARIOX_TEST_SIBLING_REMOVES";

/// Runs a presence refresh whose change is in flight while `other_writer` runs on another thread.
/// Without one writer discipline `other_writer` finishes while the refresh holds its stale copy;
/// with it, `other_writer` waits for the refresh to be saved.
fn refresh_while<T: Send>(
    refresh: impl FnOnce(&mut Vec<PersistedMachineRegistration>),
    other_writer: impl FnOnce() -> T + Send,
) -> T {
    let finished = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let other = scope.spawn(|| {
            let result = other_writer();
            finished.store(true, Ordering::SeqCst);
            result
        });
        DaemonConfig::update_machine_registry(|registry| {
            refresh(registry);
            let deadline = Instant::now() + Duration::from_millis(500);
            while !finished.load(Ordering::SeqCst) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
        })
        .expect("refresh should persist");
        other.join().expect("the other writer should not panic")
    })
}

// A refresh that loaded the approved machine must not save it back after Remove reported success.
#[test]
fn a_presence_refresh_never_brings_back_a_machine_the_owner_removed() {
    crate::test_support::isolated_env_test!();
    DaemonConfig::approve_remote_machine("machine-a").expect("approval should persist");

    let removed = refresh_while(
        |registry| registry[0].last_seen_ms = 42,
        || DaemonConfig::forget_remote_machine("machine-a", |_| {}),
    )
    .expect("Remove should persist");

    assert!(removed.is_some());
    assert!(DaemonConfig::machine_registry_entries().is_empty());
    assert!(DaemonConfig::worker_machine_revoked("machine-a"));
}

// A client revocation made while a refresh runs must survive the refresh's save.
#[test]
fn a_presence_refresh_keeps_a_client_revocation_made_meanwhile() {
    crate::test_support::isolated_env_test!();
    DaemonConfig::record_paired_client("client-a", "thumb", None, 1).expect("pairing persists");

    refresh_while(
        |registry| {
            registry.push(PersistedMachineRegistration {
                machine_id: "machine-b".into(),
                ..PersistedMachineRegistration::default()
            })
        },
        || DaemonConfig::revoke_paired_client("client-a"),
    )
    .expect("revocation should persist");

    assert!(DaemonConfig::client_pairing_entries()[0].revoked);
    assert_eq!(DaemonConfig::machine_registry_entries().len(), 1);
}

// Not a test of its own: the sibling kernel process the test below starts. It shares the
// parent's `CHARIOX_HOME`, so it is the other writer of the same `daemon/config.json`.
#[test]
fn sibling_kernel_removes_machine_a() {
    let Some(marker) = std::env::var_os(SIBLING_REMOVE) else {
        return;
    };
    std::fs::write(&marker, b"started").expect("sibling should say it started");
    DaemonConfig::forget_remote_machine("machine-a", |_| {}).expect("Remove should persist");
}

// Two kernel processes share one `CHARIOX_HOME`: the other one's Remove, made while this one's
// presence refresh is in flight, still stands after the refresh is saved.
#[test]
fn a_sibling_kernels_remove_survives_this_kernels_presence_refresh() {
    crate::test_support::isolated_env_test!();
    DaemonConfig::approve_remote_machine("machine-a").expect("approval should persist");
    let marker = std::env::temp_dir().join("sibling-started");

    let mut sibling = DaemonConfig::update_machine_registry(|registry| {
        registry[0].last_seen_ms = 42;
        let mut sibling =
            Command::new(std::env::current_exe().expect("test binary should resolve"))
                .args([
                    "config::tests::daemon_config_writers::sibling_kernel_removes_machine_a",
                    "--exact",
                    "--test-threads=1",
                ])
                .env(SIBLING_REMOVE, &marker)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("sibling kernel process should start");
        let started = Instant::now();
        while !marker.exists() && started.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(marker.exists(), "the sibling kernel should start");
        // Unlocked, the sibling finishes its whole Remove here; locked, it waits for this save.
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline && sibling.try_wait().expect("sibling state").is_none() {
            std::thread::sleep(Duration::from_millis(5));
        }
        sibling
    })
    .expect("refresh should persist");

    assert!(sibling.wait().expect("sibling should end").success());
    assert!(DaemonConfig::machine_registry_entries().is_empty());
    assert!(DaemonConfig::worker_machine_revoked("machine-a"));
}
