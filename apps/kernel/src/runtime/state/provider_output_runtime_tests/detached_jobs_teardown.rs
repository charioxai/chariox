//! A job an agent's tool detached ends when its Room ends or is deleted, the
//! agent is deleted or the kernel stops, even though the agent's provider
//! process is removed after the sweep of what its owners left running.
use super::*;
use std::time::Duration;

enum Teardown {
    EndRoom,
    DeleteRoom,
    DeleteAgent,
    StopKernel,
}

use crate::test_support::process_alive as alive;

async fn the_detached_job_ends_with(teardown: Teardown, name: &str) {
    let worktree = crate::test_support::TestWorktree::new(name);
    let pid_file = worktree.path().join("detached.pid");
    let mut app =
        crate::test_support::bootstrap_authenticated_app(crate::DaemonConfig::for_tests())
            .expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt",
    )
    .with_agent_id(agent.id());
    // The subshell outlives the job's start, so the resource watch sees the
    // job before it leaves for good.
    let mut run = crate::provider::RuntimeProviderRun::new(
        format!("{name}-run"),
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: name.to_string(),
            pty_target: None,
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec![
                "-c".to_string(),
                format!(
                    r#"(nohup {} </dev/null >/dev/null 2>&1 & sleep 1)
while IFS= read -r line; do :; done"#,
                    crate::test_support::detached_job("\"$DETACHED\"", 8895),
                ),
            ],
            pty_env: [("DETACHED".to_string(), pid_file.display().to_string())].into(),
            pty_env_remove: Vec::new(),
            working_directory: Some(worktree.path().to_path_buf()),
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.pty_mut()
        .spawn_for_run(&run)
        .expect("the provider process starts");
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;

    let deadline = Instant::now() + Duration::from_secs(5);
    let job = loop {
        if let Some(pid) = std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|text| text.trim().parse::<i32>().ok())
            .filter(|pid| *pid > 1)
        {
            break pid;
        }
        assert!(Instant::now() < deadline, "the detached job did not start");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    // Past the subshell's exit: the job has left the provider's tree.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let started = alive(job);

    match teardown {
        Teardown::EndRoom => {
            runtime
                .end_session(session.id())
                .await
                .expect("the Room ends");
        }
        Teardown::DeleteRoom => {
            runtime
                .delete_session_id(session.id())
                .await
                .expect("the Room is deleted");
        }
        Teardown::DeleteAgent => {
            // Busy app: the provider process removal queues behind the sweep.
            let busy = app.lock().await;
            tokio::time::timeout(
                Duration::from_secs(20),
                runtime.destroy_agent(agent.id(), crate::session::DEFAULT_LOCAL_USER_ID),
            )
            .await
            .expect("deleting the agent does not wait for the app")
            .expect("the agent is deleted");
            drop(busy);
        }
        Teardown::StopKernel => {
            runtime.shutdown_cleanup().await.expect("the kernel stops");
        }
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while alive(job) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let survived = alive(job);
    // SAFETY: the positive PID of the job this test started; never a group.
    unsafe { libc::kill(job, libc::SIGKILL) };
    assert!(started, "the detached job did not run");
    assert!(!survived, "the agent's detached job outlived its owner");
}

#[tokio::test]
async fn a_detached_job_ends_with_its_room() {
    the_detached_job_ends_with(Teardown::EndRoom, "teardown-end-room").await;
}

#[tokio::test]
async fn a_detached_job_ends_with_its_deleted_room() {
    the_detached_job_ends_with(Teardown::DeleteRoom, "teardown-delete-room").await;
}

#[tokio::test]
async fn a_detached_job_ends_with_its_deleted_agent() {
    the_detached_job_ends_with(Teardown::DeleteAgent, "teardown-delete-agent").await;
}

#[tokio::test]
async fn a_detached_job_ends_with_the_kernel() {
    the_detached_job_ends_with(Teardown::StopKernel, "teardown-stop-kernel").await;
}
