use super::*;

pub(super) fn open(service: &RoomExecService, script: &str) -> String {
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

pub(super) async fn collect(
    service: &RoomExecService,
    id: &str,
) -> (Vec<u8>, Vec<u8>, ExecOutcome) {
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
    tokio::time::timeout(Duration::from_secs(5), async {
        while service.has_running_work() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let (_, _, outcome) = tokio::time::timeout(Duration::from_secs(5), collect(&service, &id))
        .await
        .unwrap();
    assert_eq!(
        outcome,
        ExecOutcome::Failed {
            message: "command canceled before its output ended; the stream is incomplete".into(),
        }
    );
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

// A job that leaves the command's tree (its own session, its parent gone) before anything
// looked at the tree is still the command's: cleanup owns it from the spawn, not from a snapshot.
// The second command's launcher exits at once, so it is usually gone before ownership is
// captured and its environment cannot name the claim.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn exec_cleanup_ends_a_job_that_left_the_tree_before_any_snapshot() {
    for script in [
        "(setsid sh -c 'echo $$; exec sleep 60' &) ; sleep 0.3; wait",
        "(setsid sh -c 'echo $$; exec sleep 60' &)",
    ] {
        ends_with_its_command(script).await;
    }
}

#[cfg(target_os = "linux")]
async fn ends_with_its_command(script: &str) {
    let service = RoomExecService::default();
    let id = open(&service, script);
    let page = service
        .read("room", &id, 0, Duration::from_secs(5))
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
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
    let ended = stat.as_ref().map_or(true, |stat| {
        stat.rsplit_once(')')
            .unwrap()
            .1
            .trim_start()
            .starts_with('Z')
    });
    // The test may have failed first: never leave the sleeper running.
    // SAFETY: a PID this test just read from its own child's output.
    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    assert!(
        ended,
        "the detached job must be ended with its command: {stat:?}"
    );
}

#[tokio::test]
async fn exec_cancel_never_reports_truncated_output_as_success() {
    let service = RoomExecService::default();
    // The extra pipe read fits in the OS pipe after the bounded buffer fills,
    // allowing a successful process exit while its output task stays parked.
    let directory = crate::test_support::TestWorktree::new("exec-cancel-exited-output");
    let marker = directory.path().join("exited");
    let id = open(
        &service,
        &format!("head -c 1081344 /dev/zero; : > '{}'", marker.display()),
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while !marker.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    service.cancel("room", &id).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while service.has_running_work() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let (stdout, _, outcome) = tokio::time::timeout(Duration::from_secs(5), collect(&service, &id))
        .await
        .unwrap();
    assert!(stdout.len() < 1081344);
    assert!(
        matches!(outcome, ExecOutcome::Failed { ref message } if message.contains("incomplete")),
        "{outcome:?}"
    );
    service.close("room", &id).unwrap();
}

async fn abandoned_reader_expires(script: &str) -> ExecOutcome {
    let service = RoomExecService::default();
    let id = open(&service, script);
    let record = service.record("room", &id).unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(service.has_running_work());
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(301)).await;
    // Let real OS child reaping run on wall time after the lease expires.
    tokio::time::resume();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        while service.has_running_work() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    let gone = service.record("room", &id).is_err();
    let idle = !service.has_running_work();
    // Clean up even on the unfixed implementation.
    service.end_session("room");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        gone,
        "abandoned {script} still occupies a record after five minutes"
    );
    assert!(idle, "abandoned {script} still counts as managed work");
    let outcome = record.output.lock().unwrap().outcome.clone().unwrap();
    outcome
}

#[tokio::test]
async fn exec_abandoned_stdin_reader_expires_and_releases_managed_work() {
    abandoned_reader_expires("cat").await;
}

#[tokio::test]
async fn exec_abandoned_output_reader_expires_and_releases_managed_work() {
    let outcome = abandoned_reader_expires("head -c 8388608 /dev/zero").await;
    assert_eq!(
        outcome,
        ExecOutcome::Failed {
            message: "command expired before its output ended; the stream is incomplete".into(),
        }
    );
}

#[tokio::test]
async fn exec_list_is_room_scoped_and_reports_retained_finished_output() {
    let service = RoomExecService::default();
    let id = open(&service, "printf abc");
    tokio::time::timeout(Duration::from_secs(5), async {
        while service.has_running_work() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(service.list("other-room").is_empty());
    let rows = service.list("room");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].exec_id, id);
    assert_eq!(rows[0].state, ExecState::Finished);
    assert_eq!(rows[0].buffered_bytes, 3);
    let (stdout, _, outcome) = collect(&service, &id).await;
    assert_eq!(stdout, b"abc");
    assert_eq!(outcome, ExecOutcome::Exited { code: 0 });
    service.close("room", &id).unwrap();
    assert!(service.list("room").is_empty());
}

#[tokio::test]
async fn exec_reads_renew_the_lease_but_listing_does_not() {
    let service = RoomExecService::default();
    let id = open(&service, "cat");
    tokio::time::sleep(Duration::from_millis(100)).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(299)).await;
    service.read("room", &id, 0, Duration::ZERO).await.unwrap();
    tokio::time::advance(Duration::from_secs(299)).await;
    assert_eq!(service.list("room").len(), 1, "read renewed the lease");
    tokio::time::advance(Duration::from_secs(2)).await;
    tokio::time::resume();
    tokio::time::timeout(Duration::from_secs(5), async {
        while service.has_running_work() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(service.list("room").is_empty(), "listing did not renew it");
}

#[tokio::test]
async fn exec_old_long_poll_completion_does_not_extend_an_abandoned_reader() {
    let service = RoomExecService::default();
    let id = open(&service, "cat");
    tokio::time::sleep(Duration::from_millis(100)).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(299)).await;
    let reader = service.clone();
    let exec_id = id.clone();
    let reading = tokio::spawn(async move {
        reader
            .read("room", &exec_id, 0, Duration::from_secs(240))
            .await
            .unwrap()
    });
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(240)).await;
    assert!(reading.await.unwrap().outcome.is_none());
    assert_eq!(service.list("room").len(), 1);
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::time::resume();
    tokio::time::timeout(Duration::from_secs(5), async {
        while service.has_running_work() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(service.list("room").is_empty());
}

#[tokio::test(start_paused = true)]
async fn offload_job_reader_grace_is_ten_minutes_and_normal_exec_stays_five() {
    let directory = crate::test_support::TestWorktree::new("offload-job-reader-grace");
    let service = super::queue_tests::limited(0, u64::MAX / 2);
    let normal = service
        .open("room", directory.path(), &["/bin/true".into()])
        .unwrap();
    let job = service
        .open_job_as(
            "room",
            crate::session::DEFAULT_LOCAL_USER_ID,
            directory.path(),
            &["/bin/true".into()],
            &crate::local::JobNeeds {
                memory_gib: Some(1),
                cpus: None,
            },
            None,
            Some("agent".into()),
        )
        .unwrap();
    super::queue_tests::queued(&service, &normal).await;
    super::queue_tests::queued(&service, &job).await;
    tokio::time::advance(Duration::from_secs(301)).await;
    tokio::task::yield_now().await;
    assert!(!service
        .list("room")
        .iter()
        .any(|record| record.exec_id == normal));
    assert!(service
        .list("room")
        .iter()
        .any(|record| record.exec_id == job));
    tokio::time::advance(Duration::from_secs(300)).await;
    tokio::task::yield_now().await;
    assert!(!service
        .list("room")
        .iter()
        .any(|record| record.exec_id == job));
}

#[tokio::test]
async fn offload_job_declared_memory_queues_before_creating_a_process() {
    let directory = crate::test_support::TestWorktree::new("offload-job-minimum-memory");
    let marker = directory.path().join("started");
    let service = super::queue_tests::limited(512 * 1024 * 1024, u64::MAX / 2);
    let id = service
        .open_job_as(
            "room",
            crate::session::DEFAULT_LOCAL_USER_ID,
            directory.path(),
            &[
                "/bin/sh".into(),
                "-c".into(),
                format!("touch '{}'", marker.display()),
            ],
            &crate::local::JobNeeds {
                memory_gib: Some(1),
                cpus: None,
            },
            None,
            Some("agent".into()),
        )
        .unwrap();
    super::queue_tests::queued(&service, &id).await;
    let queued = service
        .list("room")
        .into_iter()
        .find(|record| record.exec_id == id)
        .unwrap();
    assert!(
        matches!(queued.state, ExecState::Queued { wait } if wait.need_bytes == 1024 * 1024 * 1024)
    );
    assert!(!marker.exists());
    service.cancel("room", &id).unwrap();
    let (_, _, outcome) = collect(&service, &id).await;
    assert!(matches!(outcome, ExecOutcome::Failed { .. }));
    assert!(!marker.exists());
}

/// The pool stops a command it sees growing; one that already holds its memory
/// at the first sample only waits, so the memory arrives over several samples.
const GROWING_PAST_THE_POOL: &str = "import time\nx = []\nfor _ in range(16):\n    x.append(bytearray(32 * 1024 * 1024))\n    time.sleep(0.2)\ntime.sleep(30)";

#[tokio::test]
async fn offload_job_memory_stop_retains_typed_reason_after_process_and_pipes_settle() {
    let directory = crate::test_support::TestWorktree::new("offload-job-memory-outcome");
    let service = super::queue_tests::limited(96 * 1024 * 1024, u64::MAX / 2);
    let id = service
        .open_job_as(
            "room",
            crate::session::DEFAULT_LOCAL_USER_ID,
            directory.path(),
            &["python3".into(), "-c".into(), GROWING_PAST_THE_POOL.into()],
            &crate::local::JobNeeds::default(),
            None,
            Some("agent".into()),
        )
        .unwrap();
    let input = serde_json::json!({"argv":["python3", "-c", GROWING_PAST_THE_POOL], "env":{}, "job_id":"job-memory-outcome"}).to_string() + "\n";
    let offset = service
        .write(
            "room",
            &id,
            0,
            &base64::engine::general_purpose::STANDARD.encode(input.as_bytes()),
        )
        .unwrap();
    service.close_stdin("room", &id, offset).unwrap();
    let (_, _, outcome) = tokio::time::timeout(Duration::from_secs(15), collect(&service, &id))
        .await
        .expect("the command grew past the pool and was never stopped for memory");
    assert!(
        matches!(outcome, ExecOutcome::MemoryLimit { limit_bytes } if limit_bytes > 0),
        "{outcome:?}"
    );
    assert!(!service.has_running_work());
    service.close("room", &id).unwrap();
}

/// Limits the command's own domain (a `process-*` leaf the kernel made) to 128 MiB, then
/// allocates 512 MiB: the memory controller, not the kernel's pool, kills it.
fn memory_controller_kill(after: &str) -> String {
    format!(
        "cg=/sys/fs/cgroup$(sed -n 's/^0:://p' /proc/self/cgroup); \
         case \"$cg\" in */process-*) ;; *) echo \"no command domain: $cg\"; exit 3;; esac; \
         unit=$(dirname $cg); \
         echo 0 > $unit/memory.swap.max; echo 134217728 > $unit/memory.max; {after}"
    )
}

async fn killed_by_the_memory_controller(after: &str) -> (String, ExecOutcome) {
    let service = RoomExecService::default();
    let id = open(&service, &memory_controller_kill(after));
    let (_, stderr, outcome) =
        tokio::time::timeout(Duration::from_secs(20), collect(&service, &id))
            .await
            .unwrap();
    service.close("room", &id).unwrap();
    (String::from_utf8_lossy(&stderr).into_owned(), outcome)
}

#[tokio::test]
#[ignore = "sets memory.max on a cgroup unit: run with --ignored inside a throwaway delegated cgroup"]
async fn a_command_the_memory_controller_killed_ends_as_memory_limit_with_the_reason() {
    let (stderr, outcome) = killed_by_the_memory_controller(
        "exec python3 -c 'x=bytearray(512*1024*1024); print(len(x))'",
    )
    .await;
    assert!(
        matches!(outcome, ExecOutcome::MemoryLimit { limit_bytes } if limit_bytes > 0 && limit_bytes <= 134217728),
        "{outcome:?} {stderr}"
    );
    assert!(stderr.contains("killed: out of memory"), "{stderr}");
}

#[tokio::test]
#[ignore = "sets memory.max on a cgroup unit: run with --ignored inside a throwaway delegated cgroup"]
async fn a_command_that_fails_after_the_memory_controller_killed_its_child_ends_as_memory_limit() {
    let (stderr, outcome) = killed_by_the_memory_controller(
        "python3 -c 'x=bytearray(512*1024*1024); print(len(x))'; exit 101",
    )
    .await;
    assert!(
        matches!(outcome, ExecOutcome::MemoryLimit { .. }),
        "{outcome:?} {stderr}"
    );
    assert!(stderr.contains("killed: out of memory"), "{stderr}");
}

#[tokio::test]
async fn a_signalled_command_names_the_signal() {
    let service = RoomExecService::default();
    let id = open(&service, "kill -TERM $$");
    let (_, stderr, outcome) =
        tokio::time::timeout(Duration::from_secs(10), collect(&service, &id))
            .await
            .unwrap();
    assert!(
        matches!(outcome, ExecOutcome::Signaled { signal: 15 }),
        "{outcome:?}"
    );
    assert!(String::from_utf8_lossy(&stderr).contains("killed by signal 15 (SIGTERM)"));
    service.close("room", &id).unwrap();
}

#[tokio::test]
async fn offload_exec_close_stdin_after_fast_exit_still_drains_outcome() {
    let service = RoomExecService::default();
    let id = open(&service, "sleep 0.1; printf done; exit 7");
    tokio::time::timeout(Duration::from_secs(5), async {
        while service
            .list("room")
            .iter()
            .all(|row| row.state != ExecState::Running)
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    // Hold cleanup after the real child's stdin receiver closes, before finish
    // removes its sender. This is the same interval a remote close can arrive in.
    let (held_tx, held_rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        crate::runtime::process_resources::with_registry_held(|| {
            held_tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(1500));
        });
    });
    held_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let record = service.record("room", &id).unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if record
                .input
                .lock()
                .unwrap()
                .sender
                .as_ref()
                .is_some_and(|sender| sender.is_closed())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let closed = service.close_stdin("room", &id, 0);
    holder.join().unwrap();
    assert_eq!(closed.unwrap(), 0);
    let (stdout, stderr, outcome) = collect(&service, &id).await;
    assert_eq!(stdout, b"done");
    assert!(stderr.is_empty());
    assert_eq!(outcome, ExecOutcome::Exited { code: 7 });
    service.close("room", &id).unwrap();
}

#[tokio::test]
async fn a_closed_exec_leaves_no_detached_job_running() {
    let root = std::env::temp_dir().join(format!("exec-detached-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let pid_file = root.join("exec.pid");
    let service = RoomExecService::default();
    let id = open(
        &service,
        &format!(
            "nohup {} </dev/null >/dev/null 2>&1 &",
            crate::test_support::detached_job(&format!("'{}'", pid_file.display()), 8897),
        ),
    );
    let (_, _, outcome) = collect(&service, &id).await;
    assert_eq!(outcome, ExecOutcome::Exited { code: 0 });
    let read_pid = || {
        std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|text| text.trim().parse::<i32>().ok())
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while read_pid().is_none() && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let pid = read_pid().expect("the detached job started");
    service.close("room", &id).unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let alive = crate::test_support::process_alive(pid);
    // SAFETY: the positive PID of the job this test started; never a group.
    unsafe { libc::kill(pid, libc::SIGKILL) };
    std::fs::remove_dir_all(&root).ok();
    assert!(!alive, "a detached job outlived its closed exec");
}
