//! MP-08 / MP-09 / MP-10 / MP-11: `chariox.file.copy` from the tool call to the worker's answer:
//! the home agent's call goes over the connected relay to the leased agent's kernel, which
//! serves it with the real worker code, and the model gets back only the size and digest.
use super::leased_worker_route_tests::{leased_home, LeasedHome};
use crate::agent_file_copy::Workspace;
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};
use sha2::{Digest, Sha256};

const CALLER_TOKEN: &str = "file-copy-caller-bearer";

/// The home's calling agent, running in its own folder of the leased agent's session.
fn caller(home: &LeasedHome, folder: &std::path::Path) {
    let caller = home
        .state
        .owned
        .agent_store
        .create_agent(
            crate::agent::CreateAgentRequest::new(&home.session_id, "dev-stub"),
            &mut home.state.owned.session_store.write(),
        )
        .unwrap();
    let launch = LaunchProviderRequest::new(
        &home.session_id,
        "dev-stub",
        "dev-stub",
        "default",
        "default",
    )
    .with_agent_id(caller.id());
    let mut run = RuntimeProviderRun::new(
        "file-copy-caller-run",
        &launch,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "metadata-only".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: Some(folder.to_owned()),
            structured_endpoint: None,
        },
    );
    run.mark_running();
    run.set_runtime_mcp_auth_token(Some(CALLER_TOKEN.into()));
    home.state
        .owned
        .provider_store
        .write()
        .insert_run_for_test(run.clone());
    home.state.owned.provider_run_projection.update(run);
}

/// Calls the tool while the worker answers up to `requests` requests from `worker`.
async fn copy(
    home: &mut LeasedHome,
    worker: &Workspace,
    requests: usize,
    arguments: serde_json::Value,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    let call = home.state.dispatch_authenticated_runtime_tool_call(
        CALLER_TOKEN,
        super::file_copy_tool::FILE_COPY,
        arguments,
    );
    tokio::pin!(call);
    let mut answered = 0;
    loop {
        tokio::select! {
            result = &mut call => return result.unwrap(),
            _ = home.rig.answer(|request| super::leased_agent_file_serve::serve(worker, request.clone())),
                if answered < requests => answered += 1,
        }
    }
}

#[tokio::test]
async fn a_home_agent_copies_a_file_to_and_from_its_leased_agent_and_sees_only_its_digest() {
    let mut home = leased_home("file-copy-tool").await;
    let folder = crate::test_support::TestWorktree::new("file-copy-caller");
    let worker_folder = crate::test_support::TestWorktree::new("file-copy-worker");
    let worker = Workspace {
        root: worker_folder.path().to_owned(),
        read_only: false,
    };
    caller(&home, folder.path());
    let bytes: Vec<u8> = (0..(4 * 1024 * 1024 + 4096))
        .map(|i: usize| (i % 251) as u8)
        .collect();
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    std::fs::create_dir_all(worker.root.join("out")).unwrap();
    std::fs::write(worker.root.join("out/x.bundle"), &bytes).unwrap();
    let agent = home.agent_id.clone();

    let pulled = copy(
        &mut home,
        &worker,
        2,
        serde_json::json!({
            "source": {"agent": agent, "path": "out/x.bundle"},
            "destination": {"path": "inbox/x.bundle"},
        }),
    )
    .await;
    assert!(pulled.ok, "{}", pulled.payload);
    assert_eq!(
        pulled.payload,
        serde_json::json!({
            "bytes": bytes.len(),
            "sha256": sha256,
            "source": {"agent": agent, "path": "out/x.bundle"},
            "destination": {"path": "inbox/x.bundle"},
        })
    );
    assert_eq!(
        std::fs::read(folder.path().join("inbox/x.bundle")).unwrap(),
        bytes
    );

    let pushed = copy(
        &mut home,
        &worker,
        2,
        serde_json::json!({
            "source": {"path": "inbox/x.bundle"},
            "destination": {"agent": agent, "path": "in/base.bundle"},
            "expect_sha256": sha256,
        }),
    )
    .await;
    assert!(pushed.ok, "{}", pushed.payload);
    assert_eq!(pushed.payload["sha256"], sha256);
    assert_eq!(
        std::fs::read(worker.root.join("in/base.bundle")).unwrap(),
        bytes
    );

    // The worker's refusal reaches the model as the reason, and the home discards what was staged.
    let hook = copy(
        &mut home,
        &worker,
        2,
        serde_json::json!({
            "source": {"path": "inbox/x.bundle"},
            "destination": {"agent": agent, "path": ".git/hooks/pre-commit"},
        }),
    )
    .await;
    assert!(!hook.ok);
    assert!(
        hook.payload["error"].as_str().unwrap().contains(".git"),
        "{}",
        hook.payload
    );
    assert!(!worker.root.join(".git").exists());

    for arguments in [
        serde_json::json!({"source": {"path": "a"}, "destination": {"path": "b"}}),
        serde_json::json!({"source": {"agent": agent, "path": "a"}, "destination": {"agent": agent, "path": "b"}}),
    ] {
        let refused = copy(&mut home, &worker, 0, arguments).await;
        assert!(
            refused.payload["error"]
                .as_str()
                .unwrap()
                .contains("exactly one side"),
            "{}",
            refused.payload
        );
    }
}
