//! MP-08 / MP-10 / MP-11: an unused Claude session ID is not a persisted conversation.
use super::*;
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, ProviderResumeState,
};

fn restarted_session_arguments(existing: Option<&str>, submitted: bool) -> (String, String) {
    let worktree = crate::test_support::TestWorktree::new("claude-cold-restart");
    let trace = worktree.path().join("argv.log");
    let mut request = LaunchProviderRequest::new("room", "claude", "claude", "default", "sonnet");
    if let Some(session) = existing {
        request = request.with_resume_state(ProviderResumeState::from_claude_session_id(session));
    }
    let run = RuntimeProviderRun::new(
        "cold-restart-run",
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "claude-argv-regression".into(),
            pty_target: None,
            pty_program: Some("/bin/sh".into()),
            pty_args: vec![
                "-c".into(),
                r#"printf 'argv' >> "$CLAUDE_TEST_TRACE"
for arg in "$@"; do printf '\t%s' "$arg" >> "$CLAUDE_TEST_TRACE"; done
printf '\n' >> "$CLAUDE_TEST_TRACE"
session_id=''; previous=''
for arg in "$@"; do
  case "$previous" in --session-id|--resume) session_id="$arg" ;; esac
  previous="$arg"
done
while IFS= read -r line; do printf '{"type":"system","subtype":"init","session_id":"%s"}\n' "$session_id"; done"#
                    .into(),
                "fixture".into(),
            ],
            pty_env: BTreeMap::from([("CLAUDE_TEST_TRACE".into(), trace.display().to_string())]),
            pty_env_remove: Vec::new(),
            working_directory: Some(worktree.path().to_path_buf()),
            structured_endpoint: None,
        },
    );
    let mut binding = initialize_claude_runtime(&run).unwrap();
    let session = binding
        .state
        .session_id()
        .map(str::to_string)
        .unwrap_or_else(|| {
            binding
                .state
                .args
                .windows(2)
                .find(|pair| pair[0] == "--session-id")
                .unwrap()[1]
                .clone()
        });
    wait_for_launches(&trace, 1);
    if submitted {
        let envelope = crate::prompt_assembly::PromptEnvelope::new(
            "real turn",
            "",
            Vec::new(),
            crate::prompt_assembly::PromptManifest::current(),
        );
        submit_claude_prompt(&run, &mut binding.state, &envelope).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while binding.state.session_id().is_none() {
            super::drain_claude_events(&run, &mut binding.state).unwrap();
            assert!(
                Instant::now() < deadline,
                "official init must confirm a resumable session"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    restart_claude_runtime(&run, &mut binding.state, "regression_restart_selection").unwrap();
    let contents = wait_for_launches(&trace, 2);
    (contents.lines().last().unwrap().to_string(), session)
}

fn wait_for_launches(path: &std::path::Path, count: usize) -> String {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let contents = std::fs::read_to_string(path).unwrap_or_default();
        if contents.ends_with('\n') && contents.lines().count() >= count {
            return contents;
        }
        assert!(Instant::now() < deadline, "child launch trace missing");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn selection_restart_before_first_prompt_keeps_a_new_session() {
    let (args, session) = restarted_session_arguments(None, false);
    assert!(
        args.ends_with(&format!("\t--session-id\t{session}")),
        "an unused session must start with --session-id, never --resume: {args}"
    );
    assert!(!args.contains("\t--resume\t"));
}

#[test]
fn first_selection_restart_can_resume_an_existing_conversation() {
    let (args, session) = restarted_session_arguments(Some("persisted-conversation"), false);
    assert!(args.ends_with(&format!("\t--resume\t{session}")));
}

#[test]
fn selection_restart_after_prompt_submission_preserves_the_session() {
    let (args, session) = restarted_session_arguments(None, true);
    assert!(args.ends_with(&format!("\t--resume\t{session}")));
}

// A job an agent's tool detached (`setsid nohup job &`) is still the agent's
// after Claude restarts: running, listed under the agent, and stoppable there.
#[test]
fn a_job_the_agent_detached_outlives_a_claude_restart_and_stays_its_own() {
    let worktree = crate::test_support::TestWorktree::new("claude-detached-job");
    let detached = worktree.path().join("detached.pid");
    let attached = worktree.path().join("attached.pid");
    // A tool command Claude still runs, in a session of its own.
    let tool = worktree.path().join("tool.pid");
    let request =
        LaunchProviderRequest::new("detach-room", "claude", "claude", "default", "sonnet")
            .with_agent_id("detach-agent");
    let run = RuntimeProviderRun::new(
        "detach-run",
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "claude-detached-job".into(),
            pty_target: None,
            pty_program: Some("/bin/sh".into()),
            pty_args: vec![
                "-c".into(),
                r#"if [ ! -e "$DETACHED" ]; then
  (setsid nohup sh -c 'echo $$ > "$DETACHED"; exec sleep 8891' </dev/null >/dev/null 2>&1 &)
  sh -c 'echo $$ > "$ATTACHED"; exec sleep 8892' </dev/null >/dev/null 2>&1 &
  setsid sh -c 'echo $$ > "$TOOL"; exec sleep 8895' </dev/null >/dev/null 2>&1 &
fi
while IFS= read -r line; do :; done"#
                    .into(),
            ],
            pty_env: BTreeMap::from([
                ("DETACHED".into(), detached.display().to_string()),
                ("ATTACHED".into(), attached.display().to_string()),
                ("TOOL".into(), tool.display().to_string()),
            ]),
            pty_env_remove: Vec::new(),
            working_directory: Some(worktree.path().to_path_buf()),
            structured_endpoint: None,
        },
    );
    let mut binding = initialize_claude_runtime(&run).unwrap();
    let pid = |path: &std::path::Path| {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| text.trim().parse::<i32>().ok())
            .filter(|pid| *pid > 1)
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while pid(&detached).is_none() || pid(&attached).is_none() || pid(&tool).is_none() {
        assert!(Instant::now() < deadline, "the agent's jobs did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    // Let the resource watch observe both jobs, as it would during a turn.
    std::thread::sleep(Duration::from_millis(600));
    let (job, leftover, command) = (
        pid(&detached).unwrap(),
        pid(&attached).unwrap(),
        pid(&tool).unwrap(),
    );
    restart_claude_runtime(&run, &mut binding.state, "regression_restart_detached").unwrap();
    std::thread::sleep(Duration::from_millis(600));
    // SAFETY: signal 0 only probes the positive PIDs this test started.
    let alive = |pid: i32| unsafe { libc::kill(pid, 0) } == 0 && !zombie(pid);
    let (job_alive, leftover_alive, command_alive) = (alive(job), alive(leftover), alive(command));
    let owner =
        crate::runtime::process_resources::SessionProcessOwner::Agent("detach-agent".into());
    let listed = crate::runtime::process_resources::released_keys_for_session("detach-room")
        .into_iter()
        .filter(|(row_owner, _)| *row_owner == owner)
        .flat_map(|(_, key)| crate::runtime::process_resources::samples_for_key(&key))
        .any(|row| row.pid == job as u32);
    crate::runtime::process_resources::stop_released(Some("detach-room"), Some(&owner));
    // A stopped process may still be on its way out for a moment.
    let deadline = Instant::now() + Duration::from_secs(2);
    while alive(job) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let stopped = !alive(job);
    for pid in [job, leftover, command] {
        // SAFETY: the positive PIDs of the jobs this test started; never a group.
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    assert!(
        job_alive,
        "a job the agent detached ended with the Claude restart"
    );
    assert!(
        !leftover_alive,
        "a job left in Claude's session survived the restart"
    );
    assert!(
        !command_alive,
        "a tool command Claude still ran survived the restart"
    );
    assert!(listed, "the detached job is not listed under its agent");
    assert!(
        stopped,
        "stopping the agent's released processes left the job running"
    );
}

// An agent restarted twice is still one accounting unit: what each run started
// is limited together, not once per run.
#[test]
fn an_agent_restarted_twice_stays_one_limited_unit() {
    let worktree = crate::test_support::TestWorktree::new("claude-unit-restarts");
    let request = LaunchProviderRequest::new("unit-room", "claude", "claude", "default", "sonnet")
        .with_agent_id("unit-agent");
    let run = RuntimeProviderRun::new(
        "unit-run",
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "claude-unit-restarts".into(),
            pty_target: None,
            pty_program: Some("/bin/sh".into()),
            pty_args: vec![
                "-c".into(),
                r#"(setsid nohup sh -c 'echo $$ > "$JOBS/job.$$"; exec sleep 8896' </dev/null >/dev/null 2>&1 &)
while IFS= read -r line; do :; done"#
                    .into(),
            ],
            pty_env: BTreeMap::from([("JOBS".into(), worktree.path().display().to_string())]),
            pty_env_remove: Vec::new(),
            working_directory: Some(worktree.path().to_path_buf()),
            structured_endpoint: None,
        },
    );
    let jobs = || -> Vec<i32> {
        std::fs::read_dir(worktree.path())
            .unwrap()
            .filter_map(|entry| {
                let name = entry.ok()?.file_name().into_string().ok()?;
                name.strip_prefix("job.")?.parse().ok()
            })
            .collect()
    };
    let wait_for_jobs = |count: usize| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while jobs().len() < count {
            assert!(Instant::now() < deadline, "the run's job did not start");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let mut binding = initialize_claude_runtime(&run).unwrap();
    wait_for_jobs(1);
    std::thread::sleep(Duration::from_millis(600));
    restart_claude_runtime(&run, &mut binding.state, "regression_unit_first").unwrap();
    wait_for_jobs(2);
    std::thread::sleep(Duration::from_millis(600));
    restart_claude_runtime(&run, &mut binding.state, "regression_unit_second").unwrap();
    wait_for_jobs(3);
    // The Chariox cgroups that carry a memory limit above each run's job.
    let limited = jobs()
        .into_iter()
        .flat_map(|pid| {
            let own = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap_or_default();
            let mut path = std::path::Path::new("/sys/fs/cgroup")
                .join(own.trim().strip_prefix("0::/").unwrap_or_default());
            let mut found = Vec::new();
            while path.starts_with("/sys/fs/cgroup")
                && path != std::path::Path::new("/sys/fs/cgroup")
            {
                let ours = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("process-") || name.starts_with("unit-"));
                if ours
                    && std::fs::read_to_string(path.join("memory.high"))
                        .is_ok_and(|limit| limit.trim() != "max")
                {
                    found.push(path.clone());
                }
                path.pop();
            }
            found
        })
        .collect::<std::collections::BTreeSet<_>>();
    let owner = crate::runtime::process_resources::SessionProcessOwner::Agent("unit-agent".into());
    crate::runtime::process_resources::stop_released(Some("unit-room"), Some(&owner));
    for pid in jobs() {
        // SAFETY: the positive PIDs of the jobs this test started; never a group.
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    assert!(
        limited.len() <= 1,
        "the agent's three runs hold {} separately limited domains: {limited:?}",
        limited.len()
    );
}

fn zombie(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            stat.rfind(')')
                .map(|end| stat[end + 1..].trim_start().starts_with('Z'))
        })
        .unwrap_or(false)
}
