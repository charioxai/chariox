//! Separate invocations of one command series reach the kernel as separate
//! processes under the same holders; each must land on the same grant.
use super::*;

fn read_pid(path: &std::path::Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn stop(pid: u32) {
    assert!(pid > 1, "refusing to signal pid {pid}");
    let _ = std::process::Command::new("kill")
        .arg(pid.to_string())
        .status();
}

/// Starts `outer` > `inner` > `peer` outside the kernel's own process tree:
/// the launcher exits at once, so `outer` is orphaned. Returns their pids.
fn orphaned_chain(dir: &std::path::Path) -> (u32, u32, u32) {
    std::fs::write(
        dir.join("outer.sh"),
        "echo $$ > \"$D/outer\"\nsh \"$D/inner.sh\" &\nwait\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("inner.sh"),
        "echo $$ > \"$D/inner\"\nsleep 30 &\necho $! > \"$D/peer\"\nwait\n",
    )
    .unwrap();
    let status = std::process::Command::new("sh")
        .args(["-c", "sh \"$D/outer.sh\" >/dev/null 2>&1 &"])
        .env("D", dir)
        .status()
        .unwrap();
    assert!(status.success());
    for _ in 0..100 {
        if let (Some(outer), Some(inner), Some(peer)) = (
            read_pid(&dir.join("outer")),
            read_pid(&dir.join("inner")),
            read_pid(&dir.join("peer")),
        ) {
            return (outer, inner, peer);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("the helper process chain did not start");
}

#[tokio::test]
async fn a_peer_under_two_grant_holders_uses_the_nearest_holders_grant() {
    let app =
        crate::test_support::bootstrap_authenticated_app(crate::config::DaemonConfig::for_tests())
            .unwrap();
    let state = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
        32,
    )
    .runtime_state();
    let dir = std::env::temp_dir().join(format!(
        "chariox-nearest-grant-{:016x}",
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let (outer, inner, peer) = orphaned_chain(&dir);

    let identity = |pid| process::inspect(pid).unwrap().0;
    // The ids sort the far holder first, as in the field report.
    for (id, holder) in [
        ("access-a-far", identity(outer)),
        ("access-z-near", identity(inner)),
    ] {
        state.owned.kernel_access.lock().unwrap().grants.insert(
            id.into(),
            Grant {
                covered: Vec::new(),
                summary: KernelAccessGrant {
                    covered: Vec::new(),
                    grant_id: id.into(),
                    owner_user_id: "local".into(),
                    holder_pid: holder.pid,
                    holder_executable: holder.executable.clone(),
                    holder_harness: None,
                    term: KernelAccessTerm::UntilHolderExits,
                    expires_at_ms: None,
                },
                holder,
                clock: None,
            },
        );
    }
    let order: Vec<_> = state
        .access_grants_for(&identity(peer))
        .into_iter()
        .map(|grant| grant.summary.grant_id)
        .collect();
    for pid in [peer, inner, outer] {
        stop(pid);
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(order, ["access-z-near", "access-a-far"]);
}
