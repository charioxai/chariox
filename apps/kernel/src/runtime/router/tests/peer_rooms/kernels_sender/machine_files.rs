//! MP-08 / MP-09 / MP-10 / MP-11 (files r1): an agent on one kernel lists, reads, copies and moves
//! files between the machines of its owner through its Files tools, end to end: its tool call, the
//! other kernel's file service and transfer job, and the bytes on disk. The two kernels share an
//! owner; the relay between them is the in-process link.
use super::*;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// `kernel` answers file requests under its id and name, and the peer agent requests as before.
fn machine_link(kernel: &Kernel) {
    let router = kernel.router.clone();
    peer_agent_link_for_tests::register(
        &kernel.id,
        &kernel.name,
        Arc::new(
            move |from: String, owner: String, request: RelayPeerRequest| {
                let router = router.clone();
                Box::pin(async move {
                    match request {
                        RelayPeerRequest::WatchMachineFiles { .. }
                        | RelayPeerRequest::UnwatchMachineFiles { .. }
                        | RelayPeerRequest::MachineFilesChanged { .. }
                        | RelayPeerRequest::StatMachinePath { .. }
                        | RelayPeerRequest::ListMachineDirectory { .. }
                        | RelayPeerRequest::ReadMachineFile { .. }
                        | RelayPeerRequest::DeleteMachinePath { .. }
                        | RelayPeerRequest::StartFileTransfer { .. }
                        | RelayPeerRequest::GetFileTransfer { .. }
                        | RelayPeerRequest::CancelFileTransfer { .. } => {
                            router.relay_machine_files(&from, request).await
                        }
                        other => router.relay_peer_agent(&from, &owner, other).await,
                    }
                })
            },
        ),
    );
}

async fn tool(
    kernel: &Kernel,
    name: &str,
    arguments: serde_json::Value,
) -> Result<serde_json::Value, String> {
    match kernel
        .router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(&kernel.token, name, arguments)
        .await
    {
        Ok(result) if result.ok => Ok(result.payload),
        Ok(result) => Err(result.payload.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("files-tools-{name}-{:08x}", rand::random::<u32>()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn plant(path: &Path, size: usize) -> String {
    let bytes: Vec<u8> = (0..size).map(|i| (i * 31 + i / 4099) as u8).collect();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, &bytes).unwrap();
    format!("{:x}", Sha256::digest(&bytes))
}

fn digest_of(path: &Path) -> String {
    format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))
}

async fn finished(kernel: &Kernel, machine: Option<&str>, job: &str) -> serde_json::Value {
    let mut arguments = serde_json::json!({"job_id": job, "wait_seconds": 30});
    if let Some(machine) = machine {
        arguments["machine"] = machine.into();
    }
    tool(kernel, "chariox.files.transfer", arguments)
        .await
        .expect("the transfer ended well")
}

#[test]
fn an_agent_lists_reads_copies_and_moves_files_between_machines_and_sees_only_the_outcome() {
    run_peer_test("files-tools", || async {
        let (a, b) = two_kernels(
            "files-tools",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        machine_link(&a);
        machine_link(&b);
        let (on_a, on_b) = (scratch("a"), scratch("b"));
        let size = 6 * 1024 * 1024 + 77;
        let sha256 = plant(&on_b.join("logs/data.bin"), size);
        std::fs::write(on_b.join("logs/notes.txt"), "line one\nline two\n").unwrap();

        // List and read on the other machine.
        let listed = tool(
            &a,
            "chariox.files.list",
            serde_json::json!({"machine": b.name, "path": on_b.join("logs")}),
        )
        .await
        .unwrap();
        let names: Vec<_> = listed["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["data.bin", "notes.txt"]);
        assert!(listed["entries"][0]["revision"]
            .as_str()
            .is_some_and(|r| !r.is_empty()));
        let page = tool(&a, "chariox.files.read", serde_json::json!({"machine": b.name, "path": on_b.join("logs/notes.txt"), "length": 8})).await.unwrap();
        assert_eq!(
            (page["text"].as_str(), page["eof"].as_bool()),
            (Some("line one"), Some(false))
        );

        // Copy from B to A: this kernel pulls from the other; the result is metadata only.
        let copied = tool(
            &a,
            "chariox.files.copy",
            serde_json::json!({
                "source": {"machine": b.name, "path": on_b.join("logs/data.bin")},
                "destination": {"folder": on_a},
            }),
        )
        .await
        .unwrap();
        assert_eq!(copied["state"], "done", "{copied}");
        assert_eq!(copied["sha256"], sha256.as_str());
        assert_eq!(copied["bytes_done"], size);
        assert!(
            copied.to_string().len() < 1500,
            "the model gets no file bytes: {copied}"
        );
        assert_eq!(digest_of(&on_a.join("data.bin")), sha256);

        // Copy from A to B: the other kernel is told to pull from this one; B's job runs on B.
        let sent = tool(
            &a,
            "chariox.files.copy",
            serde_json::json!({
                "source": {"path": on_a.join("data.bin")},
                "destination": {"machine": b.name, "folder": on_b.join("inbox")},
                "wait_seconds": 0,
            }),
        )
        .await
        .unwrap();
        let done = finished(&a, Some(&b.name), sent["job_id"].as_str().unwrap()).await;
        assert_eq!(done["state"], "done", "{done}");
        assert_eq!(digest_of(&on_b.join("inbox/data.bin")), sha256);

        // The name is taken: the default refuses and says what to do; keep_both and move do as asked.
        let taken = tool(
            &a,
            "chariox.files.copy",
            serde_json::json!({
                "source": {"machine": b.name, "path": on_b.join("logs/data.bin")},
                "destination": {"folder": on_a},
            }),
        )
        .await
        .unwrap_err();
        assert!(
            taken.contains("already exists") && taken.contains("keep_both"),
            "{taken}"
        );
        let both = tool(
            &a,
            "chariox.files.copy",
            serde_json::json!({
                "source": {"machine": b.name, "path": on_b.join("logs/data.bin")},
                "destination": {"folder": on_a},
                "conflict": "keep_both",
                "move": true,
            }),
        )
        .await
        .unwrap();
        assert_eq!(both["state"], "done", "{both}");
        assert_eq!(digest_of(&on_a.join("data (2).bin")), sha256);
        assert!(
            !on_b.join("logs/data.bin").exists(),
            "a move deletes the source once the copy is verified"
        );
        assert!(on_b.join("logs/notes.txt").exists());

        unlink(&[&a, &b]);
    });
}

#[test]
fn the_files_tools_never_touch_a_kernels_state_and_need_the_owners_account() {
    run_peer_test("files-tools-guard", || async {
        let _env = crate::env_lock::lock();
        let (a, b) = two_kernels(
            "files-tools-guard",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        machine_link(&a);
        machine_link(&b);
        let root = scratch("guard");
        let (kernel_home, elsewhere) = (root.join("kernel-home"), root.join("elsewhere"));
        std::fs::create_dir_all(kernel_home.join("vault")).unwrap();
        std::fs::write(kernel_home.join("vault/vault.json"), "secret").unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let previous = std::env::var_os("CHARIOX_HOME");
        std::env::set_var("CHARIOX_HOME", &kernel_home);

        let refused = tool(
            &a,
            "chariox.files.list",
            serde_json::json!({"machine": b.name, "path": kernel_home}),
        )
        .await
        .unwrap_err();
        assert!(refused.contains("own state or Vault"), "{refused}");
        let refused = tool(
            &a,
            "chariox.files.read",
            serde_json::json!({"path": kernel_home.join("vault/vault.json")}),
        )
        .await
        .unwrap_err();
        assert!(refused.contains("own state or Vault"), "{refused}");
        for copy in [
            serde_json::json!({"source": {"machine": b.name, "path": kernel_home.join("vault/vault.json")}, "destination": {"folder": elsewhere}}),
            serde_json::json!({"source": {"path": kernel_home.join("vault/vault.json")}, "destination": {"machine": b.name, "folder": elsewhere}}),
            serde_json::json!({"source": {"path": elsewhere}, "destination": {"folder": kernel_home.join("vault")}}),
        ] {
            plant(&elsewhere.join("note.txt"), 10);
            let refused = tool(&a, "chariox.files.copy", copy).await.unwrap_err();
            assert!(refused.contains("own state or Vault"), "{refused}");
        }
        assert!(!elsewhere.join("vault.json").exists());
        assert_eq!(
            std::fs::read_to_string(kernel_home.join("vault/vault.json")).unwrap(),
            "secret"
        );

        // A kernel that is not signed in to the owner's account reaches no other machine.
        let c = kernel("files-tools-guard-c", AgentPermissionLevel::Yolo, false);
        let refused = tool(
            &c,
            "chariox.files.list",
            serde_json::json!({"machine": b.name, "path": elsewhere}),
        )
        .await
        .unwrap_err();
        assert!(refused.contains("not signed in"), "{refused}");

        match previous {
            Some(value) => std::env::set_var("CHARIOX_HOME", value),
            None => std::env::remove_var("CHARIOX_HOME"),
        }
        unlink(&[&a, &b]);
    });
}
