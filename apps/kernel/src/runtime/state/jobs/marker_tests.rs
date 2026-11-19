use super::*;

#[tokio::test]
async fn offload_job_output_loss_notice_survives_discarded_and_pending_lines() {
    let (state, run, task, workspace) = tests::fixture().await;
    let completed = state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.run",
            &serde_json::json!({"command":["/bin/true"],"wait_s":5}),
            &task,
        )
        .await
        .unwrap();
    let driver = state.job_wake_driver.as_ref().unwrap();
    let mut record = driver.read(completed["job_id"].as_str().unwrap()).unwrap();

    // Obtain the notice from a real unread command, rather than manufacturing a warning string.
    let execs = crate::room_exec::RoomExecService::default();
    let id = execs
        .open(
            &task.room_id,
            workspace.path(),
            &[
                "/usr/bin/head".into(),
                "-c".into(),
                "17825792".into(),
                "/dev/zero".into(),
            ],
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        while execs.has_running_work() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let page = execs
        .read(&task.room_id, &id, 0, Duration::ZERO)
        .await
        .unwrap();
    execs.close(&task.room_id, &id).unwrap();
    let marker = page
        .chunks
        .into_iter()
        .find(|chunk| chunk.stream == crate::room_exec::ExecStream::Stderr)
        .expect("shared-buffer loss marker");
    let notice = String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(&marker.data_base64)
            .unwrap(),
    )
    .unwrap();
    assert!(notice.contains("Older unread command output was dropped"));
    assert!(notice.contains("Read output promptly"));

    for length in [1024 * 1024 + 1, 1024 * 1024 - 16] {
        let mut output = super::execute::Output::new(&state, &record).unwrap();
        let chunk = |bytes: &[u8], cursor| crate::room_exec::ExecChunk {
            cursor,
            stream: crate::room_exec::ExecStream::Stdout,
            data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        };
        for page in [
            crate::room_exec::ExecOutput {
                chunks: vec![chunk(&vec![b'x'; length], 1)],
                next_cursor: 1,
                outcome: None,
            },
            crate::room_exec::ExecOutput {
                chunks: vec![marker.clone()],
                next_cursor: marker.cursor,
                outcome: None,
            },
            crate::room_exec::ExecOutput {
                chunks: vec![chunk(b"ordinary-after\n", marker.cursor + 1)],
                next_cursor: marker.cursor + 1,
                outcome: Some(crate::room_exec::ExecOutcome::Exited { code: 0 }),
            },
        ] {
            output.page(&state, &task.room_id, &page).unwrap();
        }
        let result = record.result.as_mut().unwrap();
        result.output_tail = output.tail.clone();
        result.log = output.log.clone();
        driver.save(&record).unwrap();
        let status = state
            .dispatch_job_tool(
                None,
                "chariox.jobs.status",
                &serde_json::json!({"job_id":record.job_id}),
                &task,
            )
            .await
            .unwrap();
        let saved = std::fs::read_to_string(&output.log).unwrap();
        for (source, text) in [
            ("status", status["result"]["output_tail"].as_str().unwrap()),
            ("saved log", saved.as_str()),
        ] {
            assert!(
                text.contains(notice.trim()),
                "{source} lost the eviction warning after {length} unterminated bytes"
            );
            assert!(
                text.contains("ordinary-after"),
                "{source} lost subsequent output"
            );
        }
    }
}
