//! A covered process names background processes it started; the grant then covers them
//! by identity (pid and start time) after the shell that started them is gone.
use super::*;
use crate::runtime::kernel_access::process::{ancestry, inspect};

impl Client {
    /// Starts a background process under a live shell and returns its pid.
    fn background(&mut self, label: &str) -> u32 {
        self.command(&format!("background {label}"));
        self.result()["pid"].as_u64().unwrap() as u32
    }

    fn end_shell(&mut self, label: &str) {
        self.command(&format!("end-shell {label}"));
        assert_eq!(self.result()["shell_ended"], true);
    }

    fn cover(&mut self, pid: u32) -> Value {
        self.request(serde_json::json!({"CoverKernelAccessProcess": {"pid": pid}}))
    }
}

/// True once nothing between `pid` and init is the holder: the shell is gone.
async fn orphaned(pid: u32, holder: u32) {
    timeout(Duration::from_secs(5), async {
        while ancestry(&inspect(pid).unwrap().0)
            .unwrap()
            .iter()
            .any(|process| process.pid == holder)
        {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

/// What the background process `label` got back for `request`.
async fn call(kernel: &Kernel, label: &str, request: Option<Value>) -> Value {
    let _ = std::fs::remove_file(kernel.root.join(format!("bg-{label}-result")));
    std::fs::write(
        kernel.root.join(format!("bg-{label}-go")),
        request
            .map(|request| request.to_string())
            .unwrap_or_default(),
    )
    .unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(text) =
                std::fs::read_to_string(kernel.root.join(format!("bg-{label}-result")))
            {
                break serde_json::from_str(&text).unwrap();
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

fn covered_pids(holder: &mut Client) -> Vec<u64> {
    let listed = holder.request(serde_json::json!({"ListKernelAccessGrants": {}}));
    let grants = &listed["response"]["KernelAccessGrantsListed"]["grants"];
    grants[0]["covered"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|process| process["pid"].as_u64().unwrap())
        .collect()
}

#[tokio::test]
async fn a_covered_process_keeps_a_background_process_covered_by_identity() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    let _grant = grant(&mut kernel, &mut holder).await;
    let holder_pid = holder.child.id();
    let p1 = holder.background("p1");
    let p2 = holder.background("p2");

    let covered = holder.cover(p1);
    assert!(covered["error"].is_null(), "{covered}");
    let reply = &covered["response"]["KernelAccessCovered"];
    assert_eq!(reply["process"]["pid"], p1, "{covered}");
    assert_eq!(reply["process"]["start"], inspect(p1).unwrap().0.start);
    assert_eq!(
        reply["process"]["executable"],
        inspect(p1).unwrap().0.executable
    );
    assert_eq!(covered_pids(&mut holder), vec![u64::from(p1)]);

    holder.end_shell("p1");
    holder.end_shell("p2");
    orphaned(p1, holder_pid).await;
    orphaned(p2, holder_pid).await;

    let admitted = call(&kernel, "p1", None).await;
    assert!(
        admitted["error"].is_null(),
        "covered orphan refused: {admitted}"
    );
    // Being covered adds no authority: what the holder can't do, the process can't either.
    let human_only = call(
        &kernel,
        "p1",
        Some(serde_json::json!({"RespondToInteraction": {
            "session_id": SESSION, "interaction_id": "critical-test", "choice_id": "approve", "passkey": PASSKEY
        }})),
    )
    .await;
    assert_eq!(
        human_only["error"]["code"], "kernel_access_human_only",
        "{human_only}"
    );
    let refused = call(&kernel, "p2", None).await;
    assert_eq!(
        refused["error"]["code"], "kernel_access_not_covered",
        "{refused}"
    );

    // The process exiting ends the cover at once.
    std::fs::write(kernel.root.join("bg-p1-exit"), "").unwrap();
    timeout(Duration::from_secs(2), async {
        while !covered_pids(&mut holder).is_empty() {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("cover outlived its process");
}

#[tokio::test]
async fn the_grant_ending_ends_the_cover_of_a_background_process() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    let grant_id = grant(&mut kernel, &mut holder).await;
    let p1 = holder.background("p1");
    assert!(holder.cover(p1)["error"].is_null());
    holder.end_shell("p1");
    orphaned(p1, holder.child.id()).await;
    assert!(call(&kernel, "p1", None).await["error"].is_null());

    let revoked = kernel
        .request(serde_json::json!({"RevokeKernelAccessGrant": {"grant_id": grant_id}}))
        .await;
    assert!(revoked["error"].is_null(), "{revoked}");
    let ended = timeout(Duration::from_secs(2), call(&kernel, "p1", None))
        .await
        .unwrap();
    assert_eq!(ended["error"]["code"], "kernel_access_ended", "{ended}");
}

#[tokio::test]
async fn cover_only_names_processes_the_grant_already_covers() {
    let mut kernel = Kernel::start().await;
    let mut stranger = Client::start(&kernel.root);
    let self_cover = stranger.cover(stranger.child.id());
    assert_eq!(
        self_cover["error"]["code"], "kernel_access_not_covered",
        "{self_cover}"
    );

    let mut holder = Client::start(&kernel.root);
    let _grant = grant(&mut kernel, &mut holder).await;
    let own = holder.background("own");
    let real = inspect(own).unwrap().0;
    let mut gone = Client::start(&kernel.root);
    let gone_pid = gone.child.id();
    let _ = gone.child.kill();
    let _ = gone.child.wait();
    for (target, start, code) in [
        (stranger.child.id(), None, "kernel_access_cover_outside"),
        (1, None, "kernel_access_cover_outside"),
        (
            kernel.child.id(),
            None,
            "kernel_access_cover_kernel_process",
        ),
        (
            own,
            Some(real.start + 1),
            "kernel_access_cover_identity_changed",
        ),
        (gone_pid, None, "kernel_access_cover_gone"),
    ] {
        let refused = holder.request(
            serde_json::json!({"CoverKernelAccessProcess": {"pid": target, "start": start}}),
        );
        assert_eq!(refused["error"]["code"], code, "pid {target}: {refused}");
    }
    assert!(covered_pids(&mut holder).is_empty());

    // A covered process cannot cover a process the grant does not reach.
    assert!(holder.cover(own)["error"].is_null());
    let overreach = call(
        &kernel,
        "own",
        Some(serde_json::json!({"CoverKernelAccessProcess": {"pid": stranger.child.id()}})),
    )
    .await;
    assert_eq!(
        overreach["error"]["code"], "kernel_access_cover_outside",
        "{overreach}"
    );
    assert_eq!(covered_pids(&mut holder), vec![u64::from(own)]);
}

#[tokio::test]
async fn the_covered_set_is_bounded() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    let _grant = grant(&mut kernel, &mut holder).await;
    let mut pids = Vec::new();
    for n in 0..17 {
        pids.push(holder.background(&format!("b{n}")));
    }
    for pid in &pids[..16] {
        assert!(holder.cover(*pid)["error"].is_null());
    }
    let over = holder.cover(pids[16]);
    assert_eq!(over["error"]["code"], "kernel_access_cover_limit", "{over}");
    assert_eq!(covered_pids(&mut holder).len(), 16);
}
