//! A provider process the kernel ends for a restart leaves the jobs its tools
//! detached running, under the same agent.
use super::PtyManager;
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};
use std::time::{Duration, Instant};

use crate::test_support::process_alive as alive;

fn agent_run(
    worktree: &crate::test_support::TestWorktree,
    run_id: &str,
    agent_id: &str,
    script: &str,
) -> RuntimeProviderRun {
    let request = LaunchProviderRequest::new("pty-room", "codex", "codex", "default", "gpt")
        .with_agent_id(agent_id);
    RuntimeProviderRun::new(
        run_id,
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "pty-detached-job".into(),
            pty_target: None,
            pty_program: Some("/bin/sh".into()),
            pty_args: vec!["-c".into(), script.into()],
            pty_env: [
                (
                    "DETACHED".to_string(),
                    worktree.path().join("detached.pid").display().to_string(),
                ),
                (
                    "TOOL".to_string(),
                    worktree.path().join("tool.pid").display().to_string(),
                ),
            ]
            .into(),
            pty_env_remove: Vec::new(),
            working_directory: Some(worktree.path().to_path_buf()),
            structured_endpoint: None,
        },
    )
}

fn pid(worktree: &crate::test_support::TestWorktree, name: &str) -> Option<i32> {
    std::fs::read_to_string(worktree.path().join(name))
        .ok()
        .and_then(|text| text.trim().parse::<i32>().ok())
        .filter(|pid| *pid > 1)
}

fn wait_for_pids(worktree: &crate::test_support::TestWorktree, names: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while names.iter().any(|name| pid(worktree, name).is_none()) {
        assert!(Instant::now() < deadline, "the agent's jobs did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    // Let the resource watch observe them, as it would during a turn.
    std::thread::sleep(Duration::from_millis(600));
}

/// Whether the agent's detached job is listed under it, then stopped with it.
fn listed_then_stopped(agent_id: &str, job: i32) -> (bool, bool) {
    let owner = crate::runtime::process_resources::SessionProcessOwner::Agent(agent_id.into());
    let listed = crate::runtime::process_resources::released_keys_for_session("pty-room")
        .into_iter()
        .filter(|(row_owner, _)| *row_owner == owner)
        .flat_map(|(_, key)| crate::runtime::process_resources::samples_for_key(&key))
        .any(|row| row.pid == job as u32);
    crate::runtime::process_resources::stop_released(Some("pty-room"), Some(&owner));
    let deadline = Instant::now() + Duration::from_secs(2);
    while alive(job) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    (listed, !alive(job))
}

#[test]
fn a_job_the_agents_tool_detached_outlives_its_provider_process() {
    let worktree = crate::test_support::TestWorktree::new("pty-detached-job");
    // A tool command the provider still runs sits in a session of its own.
    let run = agent_run(
        &worktree,
        "pty-detach-run",
        "pty-agent",
        &format!(
            r#"(nohup {} </dev/null >/dev/null 2>&1 &)
{} </dev/null >/dev/null 2>&1 &
while IFS= read -r line; do :; done"#,
            crate::test_support::detached_job("\"$DETACHED\"", 8898),
            crate::test_support::session_job("\"$TOOL\"", 8899),
        ),
    );
    let mut manager = PtyManager::new();
    manager
        .spawn_for_run(&run)
        .expect("the provider process starts");
    wait_for_pids(&worktree, &["detached.pid", "tool.pid"]);
    let (job, command) = (
        pid(&worktree, "detached.pid").unwrap(),
        pid(&worktree, "tool.pid").unwrap(),
    );
    manager
        .remove_process(run.id())
        .expect("the provider process ends");
    std::thread::sleep(Duration::from_millis(600));
    let (job_alive, command_alive) = (alive(job), alive(command));
    let (listed, stopped) = listed_then_stopped("pty-agent", job);
    for pid in [job, command] {
        // SAFETY: the positive PIDs of the jobs this test started; never a group.
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    assert!(
        job_alive,
        "a job the agent detached ended with its provider process"
    );
    assert!(
        !command_alive,
        "a tool command the provider still ran survived its restart"
    );
    assert!(listed, "the detached job is not listed under its agent");
    assert!(
        stopped,
        "stopping the agent's released processes left the job running"
    );
}

#[test]
fn a_job_the_agents_tool_detached_outlives_a_provider_process_that_died() {
    let worktree = crate::test_support::TestWorktree::new("pty-died-job");
    let run = agent_run(
        &worktree,
        "pty-died-run",
        "pty-died-agent",
        &format!(
            r#"(nohup {} </dev/null >/dev/null 2>&1 &)
sleep 1; exit 3"#,
            crate::test_support::detached_job("\"$DETACHED\"", 8897),
        ),
    );
    let mut manager = PtyManager::new();
    manager
        .spawn_for_run(&run)
        .expect("the provider process starts");
    wait_for_pids(&worktree, &["detached.pid"]);
    let job = pid(&worktree, "detached.pid").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !manager
        .poll_process_state(run.id())
        .is_ok_and(|state| state.is_exited())
    {
        assert!(
            Instant::now() < deadline,
            "the provider process did not end"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    manager
        .remove_process(run.id())
        .expect("the provider process is removed");
    std::thread::sleep(Duration::from_millis(600));
    let job_alive = alive(job);
    let (listed, stopped) = listed_then_stopped("pty-died-agent", job);
    // SAFETY: the positive PID of the job this test started; never a group.
    unsafe { libc::kill(job, libc::SIGKILL) };
    assert!(
        job_alive,
        "a job the agent detached ended with its dead provider process"
    );
    assert!(listed, "the detached job is not listed under its agent");
    assert!(
        stopped,
        "stopping the agent's released processes left the job running"
    );
}
