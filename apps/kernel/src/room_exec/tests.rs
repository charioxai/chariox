use super::*;

fn open(service: &RoomExecService, script: &str) -> String {
    service
        .open(
            "room",
            Path::new("/"),
            &[
                "/bin/bash".to_string(),
                "-c".to_string(),
                script.to_string(),
            ],
        )
        .unwrap()
}

async fn collect(service: &RoomExecService, id: &str) -> (Vec<u8>, Vec<u8>, ExecOutcome) {
    let mut cursor = 0;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    loop {
        let page = service
            .read("room", id, cursor, Duration::from_secs(5))
            .await
            .unwrap();
        for chunk in page.chunks {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(chunk.data_base64)
                .unwrap();
            match chunk.stream {
                ExecStream::Stdout => stdout.extend(bytes),
                ExecStream::Stderr => stderr.extend(bytes),
            }
        }
        cursor = page.next_cursor;
        if let Some(outcome) = page.outcome {
            return (stdout, stderr, outcome);
        }
    }
}

#[tokio::test]
async fn exec_binary_duplex_keeps_streams_and_exit_separate() {
    let service = RoomExecService::default();
    let id = open(&service, "cat; printf '\\377\\000err' >&2; exit 23");
    let bytes: Vec<u8> = (0..=255).cycle().take(CHUNK_BYTES * 2 + 17).collect();
    let sending = async {
        for (index, chunk) in bytes.chunks(CHUNK_BYTES).enumerate() {
            let offset = (index * CHUNK_BYTES) as u64;
            let encoded = base64::engine::general_purpose::STANDARD.encode(chunk);
            assert_eq!(
                service.write("room", &id, offset, &encoded).unwrap(),
                offset + chunk.len() as u64
            );
            assert_eq!(
                service.write("room", &id, offset, &encoded).unwrap(),
                offset + chunk.len() as u64
            );
        }
        service
            .close_stdin("room", &id, bytes.len() as u64)
            .unwrap();
    };
    let (_, (stdout, stderr, outcome)) = tokio::join!(sending, collect(&service, &id));
    assert_eq!(stdout, bytes);
    assert_eq!(stderr, b"\xff\0err");
    assert_eq!(outcome, ExecOutcome::Exited { code: 23 });
    service.close("room", &id).unwrap();
}

#[tokio::test]
async fn exec_early_output_and_interactive_input_are_not_staged() {
    let service = RoomExecService::default();
    let id = open(
        &service,
        "printf 'ready\\n'; while read -r line; do printf 'reply:%s\\n' \"$line\"; done",
    );
    let page = service
        .read("room", &id, 0, Duration::from_secs(2))
        .await
        .unwrap();
    assert!(page.outcome.is_none());
    let data = page
        .chunks
        .iter()
        .flat_map(|chunk| {
            base64::engine::general_purpose::STANDARD
                .decode(&chunk.data_base64)
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(data, b"ready\n");
    let mut cursor = page.next_cursor;
    for (offset, line) in [(0, "one\n"), (4, "two\n")] {
        service
            .write(
                "room",
                &id,
                offset,
                &base64::engine::general_purpose::STANDARD.encode(line),
            )
            .unwrap();
        let page = service
            .read("room", &id, cursor, Duration::from_secs(2))
            .await
            .unwrap();
        assert!(page.outcome.is_none());
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(&page.chunks[0].data_base64)
                .unwrap(),
            format!("reply:{line}").as_bytes()
        );
        cursor = page.next_cursor;
    }
    service.close_stdin("room", &id, 8).unwrap();
    loop {
        let page = service
            .read("room", &id, cursor, Duration::from_secs(2))
            .await
            .unwrap();
        cursor = page.next_cursor;
        if let Some(outcome) = page.outcome {
            assert_eq!(outcome, ExecOutcome::Exited { code: 0 });
            break;
        }
    }
    service.close("room", &id).unwrap();
}

#[tokio::test]
async fn exec_slow_reader_backpressures_without_dropping_bytes() {
    let service = RoomExecService::default();
    let id = open(&service, "head -c 4194304 /dev/zero");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let record = service.record("room", &id).unwrap();
    let retained = record.output.lock().unwrap().retained_bytes();
    assert!(retained > 0 && retained <= buffer::OUTPUT_BYTES);
    assert!(service.has_running_work());
    let (stdout, stderr, outcome) =
        tokio::time::timeout(Duration::from_secs(10), collect(&service, &id))
            .await
            .unwrap();
    assert_eq!(stdout.len(), 4194304);
    assert!(stdout.iter().all(|byte| *byte == 0));
    assert!(stderr.is_empty());
    assert_eq!(outcome, ExecOutcome::Exited { code: 0 });
    service.close("room", &id).unwrap();
}

#[tokio::test]
async fn exec_cancel_settles_even_with_full_output_and_blocked_stdin() {
    let service = RoomExecService::default();
    let id = open(&service, "head -c 8388608 /dev/zero; sleep 60");
    let encoded = base64::engine::general_purpose::STANDARD.encode(vec![0; CHUNK_BYTES]);
    let mut offset = 0;
    let mut saw_busy = false;
    for _ in 0..20 {
        match service.write("room", &id, offset, &encoded) {
            Ok(next) => offset = next,
            Err(DaemonError::MachineRequest { code, .. }) if code == "exec_input_busy" => {
                saw_busy = true;
                break;
            }
            other => panic!("unexpected input result: {other:?}"),
        }
    }
    assert!(saw_busy, "stdin admission must be bounded");
    tokio::time::sleep(Duration::from_millis(100)).await;
    service.cancel("room", &id).unwrap();
    let (_, _, outcome) = tokio::time::timeout(Duration::from_secs(5), collect(&service, &id))
        .await
        .unwrap();
    assert_eq!(outcome, ExecOutcome::Signaled { signal: 9 });
    assert!(!service.has_running_work());
    service.close("room", &id).unwrap();
}

#[tokio::test]
async fn exec_refuses_cross_room_ids_and_conflicting_offsets() {
    let service = RoomExecService::default();
    let id = open(&service, "cat");
    assert!(service
        .read("other-room", &id, 0, Duration::ZERO)
        .await
        .is_err());
    assert!(service.write("other-room", &id, 0, "YQ==").is_err());
    assert!(service.cancel("other-room", &id).is_err());
    assert!(service.close("other-room", &id).is_err());
    assert!(service.write("room", &id, 4, "YQ==").is_err());
    assert_eq!(service.write("room", &id, 0, "YQ==").unwrap(), 1);
    assert!(service.write("room", &id, 0, "Yg==").is_err());
    assert!(service.close_stdin("room", &id, 0).is_err());
    service.close_stdin("room", &id, 1).unwrap();
    service.close_stdin("room", &id, 1).unwrap();
    assert!(service.write("room", &id, 1, "Yg==").is_err());
    let (out, _, _) = collect(&service, &id).await;
    assert_eq!(out, b"a");
    service.close("room", &id).unwrap();
}

#[test]
fn exec_cursor_replay_is_exact_until_acknowledged_and_never_skips() {
    let mut output = buffer::Output::default();
    output.push(ExecStream::Stdout, b"hello");
    let page = output.page(0).unwrap();
    assert_eq!(output.page(0).unwrap(), page);
    assert!(output.page(2).is_err());
    assert!(output.page(1).unwrap().chunks.is_empty());
    assert!(output.page(0).is_err());
    assert_eq!(output.retained_bytes(), 0);
}

#[tokio::test]
async fn exec_real_signal_exit_is_preserved() {
    let service = RoomExecService::default();
    let id = open(&service, "kill -TERM $$");
    let (_, _, outcome) = collect(&service, &id).await;
    assert_eq!(outcome, ExecOutcome::Signaled { signal: 15 });
    service.close("room", &id).unwrap();
}

#[tokio::test]
async fn exec_room_end_cancels_the_whole_owned_process_tree() {
    let service = RoomExecService::default();
    let id = open(&service, "sleep 60 & printf '%s\\n' $!; wait");
    let page = service
        .read("room", &id, 0, Duration::from_secs(2))
        .await
        .unwrap();
    let bytes = page
        .chunks
        .iter()
        .flat_map(|chunk| {
            base64::engine::general_purpose::STANDARD
                .decode(&chunk.data_base64)
                .unwrap()
        })
        .collect::<Vec<_>>();
    let pid: u32 = String::from_utf8(bytes).unwrap().trim().parse().unwrap();
    assert!(pid > 1);
    service.end_session("room");
    tokio::time::timeout(Duration::from_secs(5), async {
        while service.has_running_work() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(service.read("room", &id, 0, Duration::ZERO).await.is_err());
    #[cfg(target_os = "linux")]
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        assert!(
            stat.rsplit_once(')')
                .unwrap()
                .1
                .trim_start()
                .starts_with('Z'),
            "owned descendant must be exited"
        );
    }
}
