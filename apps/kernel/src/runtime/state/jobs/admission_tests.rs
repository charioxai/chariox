use std::time::Duration;

/// Ordinary pipes are owned by their Room. A jobs caller with no commands there cannot close them.
#[tokio::test]
async fn job_admission_with_no_visible_commands_ignores_another_rooms_retained_pipes() {
    let (state, run, task, worktree) = super::tests::fixture().await;
    let other_room = "another-room";
    let execs = &state.owned.room_execs;
    let mut held = Vec::new();
    for _ in 0..64 {
        let id = execs
            .open_as(
                other_room,
                &task.owner_user_id,
                worktree.path(),
                &["/bin/true".into()],
                None,
                Some("another-agent".into()),
            )
            .unwrap();
        held.push(id);
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while execs
        .list(other_room)
        .iter()
        .any(|exec| exec.state != crate::room_exec::ExecState::Finished)
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "commands never ended"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!held.is_empty());
    assert!(execs.list(&task.room_id).is_empty());
    assert!(state.session_jobs(&task.room_id).is_empty());
    let started = tokio::time::Instant::now();
    let result = state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.run",
            &serde_json::json!({"command":["/bin/true"],"wait_s":10}),
            &task,
        )
        .await
        .unwrap();
    let preserved = execs.list(other_room).len();
    for id in &held {
        execs.close(other_room, id).unwrap();
    }
    assert_eq!(
        preserved,
        held.len(),
        "admission must not discard another Room's output"
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(result["result"]["outcome"], "completed", "{result}");
    assert_eq!(result["result"]["exit_code"], 0, "{result}");
}

/// jobs.status/cancel authorize one agent's jobs. A different agent's quota is not its remedy.
#[tokio::test]
async fn job_admission_with_no_owned_jobs_ignores_another_agents_active_jobs() {
    let (state, run, task, worktree) = super::tests::fixture().await;
    let execs = &state.owned.room_execs;
    let mut held = Vec::new();
    for _ in 0..64 {
        let id = execs
            .open_job_as(
                &task.room_id,
                &task.owner_user_id,
                worktree.path(),
                &["/bin/sleep".into(), "60".into()],
                &crate::local::JobNeeds::default(),
                None,
                Some("another-agent".into()),
            )
            .unwrap();
        held.push(id);
    }
    assert!(!held.is_empty());
    assert!(state.session_jobs(&task.room_id).is_empty());
    let result = state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.run",
            &serde_json::json!({"command":["/bin/true"],"wait_s":10}),
            &task,
        )
        .await
        .unwrap();
    let preserved = execs
        .list(&task.room_id)
        .into_iter()
        .filter(|exec| held.contains(&exec.exec_id))
        .count();
    for id in &held {
        execs.close(&task.room_id, id).unwrap();
    }
    assert_eq!(
        preserved,
        held.len(),
        "admission must not cancel a peer's jobs"
    );
    assert!(result["job_id"].as_str().is_some(), "{result}");
    assert_ne!(result["result"]["outcome"], "outside_end", "{result}");
}
