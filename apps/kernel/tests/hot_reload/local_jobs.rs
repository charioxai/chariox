use super::*;

#[test]
fn a_local_job_completes_once_with_its_output_after_two_hot_reloads() {
    let scratch = std::env::temp_dir().join(format!("hr-job-{}", std::process::id()));
    let record = scratch.join("record");
    let codex = scratch.join("codex");
    std::fs::create_dir_all(&record).unwrap();
    std::fs::create_dir_all(scratch.join("codex-home")).unwrap();
    install(
        |file| {
            std::fs::write(
                file,
                format!(
                    "#!/bin/sh\nexec python3 {} \"$@\"\n",
                    scratch.join("fake_codex.py").display()
                ),
            )
            .unwrap();
        },
        &codex,
    );
    std::fs::write(scratch.join("fake_codex.py"), FAKE_CODEX).unwrap();
    let kernel = start_laptop_kernel(
        &scratch,
        &[
            ("CHARIOX_CODEX_BIN", codex),
            ("CHARIOX_FAKE_CODEX_RECORD", record.clone()),
            ("CODEX_HOME", scratch.join("codex-home")),
        ],
    );
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(local_job_drill(&kernel, &record));
}

async fn local_job_drill(kernel: &Kernel, record: &Path) {
    let mut socket = connect(kernel, Duration::from_secs(60)).await;
    let project = kernel.scratch.join("project");
    let created = request(
        &mut socket,
        json!({"CreateSession": {
            "workspace_id": project, "worktree_id": project, "alias": null, "no_subscribe": true,
        }}),
    )
    .await;
    let room = created["SessionCreated"]["session"]["id"].as_str().unwrap();
    let spawned = request(
        &mut socket,
        json!({"SpawnAgent": {
            "session_id": room, "alias": "job-runner", "provider": "codex", "model": "gpt-6-luna",
            "effort": "low", "worktree_id": null, "no_subscribe": true,
        }}),
    )
    .await;
    let agent = spawned["AgentSpawned"]["agent"]["id"].as_str().unwrap();
    let attached = request(
        &mut socket,
        json!({"AttachToSession": {
            "session_id": room, "client_id": "local-job-drill", "capability_level": "FullTerminal",
        }}),
    )
    .await;
    let attachment = attached["SessionAttached"]["attachment"]["id"]
        .as_str()
        .unwrap();
    request(
        &mut socket,
        json!({"SubmitPrompt": {
            "session_id": room, "target_agent_id": agent, "attachment_id": attachment,
            "prompt": "Run the local job drill", "attachments": [],
        }}),
    )
    .await;
    wait_for("the agent opens its turn", || record.join("parts").exists()).await;
    let before = request(
        &mut socket,
        json!({"GetSessionState": { "session_id": room }}),
    )
    .await;
    let task = before["SessionState"]["session"]["prompt_states"][agent]["active_prompt"]["id"]
        .as_str()
        .unwrap();
    let started = job_tool(record, "start", "chariox.jobs.run", json!({
        "task_id": task, "origin_prompt_id": task, "timeout_s": 150, "wait_s": 1,
        "command": ["/bin/sh", "-c", "printf 'before\\npartial-'; echo $$ > started; while [ ! -f finish ]; do sleep 0.05; done; printf 'line\\nafter\\n'; exit 7"],
    })).await;
    assert_eq!(started["placement"], "local", "{started}");
    let job = started["job_id"].as_str().unwrap();
    wait_for("the local job starts", || project.join("started").exists()).await;
    let runner_pid = std::fs::read_to_string(project.join("started")).unwrap();
    let args = json!({ "task_id": task, "origin_prompt_id": task, "job_id": job });
    let running = job_tool(record, "before", "chariox.jobs.status", args.clone()).await;
    assert_eq!(running["state"], "running", "{running}");
    let armed = request(
        &mut socket,
        json!({"GetSessionState": { "session_id": room }}),
    )
    .await;
    let wake = job_wake(&armed, job);
    let wake_id = wake["id"].as_str().unwrap();
    assert_eq!(wake["fire_count"], 0, "{wake}");

    std::fs::write(record.join("swap"), "").unwrap();
    for round in 1..=2 {
        let installed = kernel.scratch.join("bin/chariox-kernel");
        install(
            |staged| {
                std::fs::copy(&installed, staged).unwrap();
            },
            &installed,
        );
        let pid = kernel.child.id() as i32;
        unsafe { libc::kill(pid, libc::SIGUSR2) };
        closed_by_the_old_program(socket).await;
        socket = connect(kernel, Duration::from_secs(60)).await;
        let after = job_tool(
            record,
            &format!("after-{round}"),
            "chariox.jobs.status",
            args.clone(),
        )
        .await;
        assert_eq!(after["state"], "running", "{after}");
        assert_eq!(
            after["registration_id"], started["registration_id"],
            "{after}"
        );
        assert_eq!(
            std::fs::read_to_string(project.join("started")).unwrap(),
            runner_pid
        );
    }

    std::fs::write(project.join("finish"), "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut poll = 0;
    let finished = loop {
        poll += 1;
        let status = job_tool(
            record,
            &format!("finished-{poll}"),
            "chariox.jobs.status",
            args.clone(),
        )
        .await;
        if status["state"] == "finished" {
            break status;
        }
        assert!(Instant::now() < deadline, "job never finished: {status}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(finished["result"]["outcome"], "completed", "{finished}");
    assert_eq!(finished["result"]["exit_code"], 7, "{finished}");
    assert_eq!(
        finished["result"]["output_tail"], "before\npartial-line\nafter\n",
        "{finished}"
    );
    let log = Path::new(finished["result"]["log"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(log).unwrap(),
        "before\npartial-line\nafter\n"
    );
    let final_state = request(
        &mut socket,
        json!({"GetSessionState": { "session_id": room }}),
    )
    .await;
    let wake = job_wake(&final_state, job);
    assert_eq!(wake["id"], wake_id, "{wake}");
    assert_eq!(wake["fire_count"], 1, "{wake}");
    let tasks = &final_state["SessionState"]["session"]["agent_tasks"];
    let task = tasks
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["task_id"] == task)
        .unwrap();
    let obligations = task["obligations"].as_array().unwrap();
    assert_eq!(obligations.len(), 1, "{task}");
    assert_eq!(obligations[0]["kind"], "job", "{task}");
    assert_eq!(obligations[0]["status"], "failed", "{task}");
    std::fs::write(record.join("go"), "").unwrap();
}

fn job_wake<'a>(state: &'a Value, job: &str) -> &'a Value {
    let mut wakes = state["SessionState"]["session"]["agent_wakes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|wake| wake["kind"] == "job");
    let wake = wakes.next().unwrap();
    assert_eq!(wake["detail"]["job_id"], job);
    assert!(wakes.next().is_none(), "job opened more than one wake");
    wake
}

async fn job_tool(record: &Path, suffix: &str, tool: &str, arguments: Value) -> Value {
    let answer = codex_tools(
        record,
        suffix,
        json!({"jsonrpc": "2.0", "id": 1,
            "method": "tools/call", "params": {"name": tool, "arguments": arguments},
        }),
    )
    .await;
    // The fixture's request directory survives a provider process replacement.
    std::fs::remove_file(record.join(format!("mcp-{suffix}"))).unwrap();
    assert!(answer["error"].is_null(), "{answer}");
    serde_json::from_str(answer["result"]["content"][0]["text"].as_str().unwrap())
        .unwrap_or_else(|_| panic!("{answer}"))
}

async fn wait_for(label: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !predicate() {
        assert!(Instant::now() < deadline, "timed out waiting for {label}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
