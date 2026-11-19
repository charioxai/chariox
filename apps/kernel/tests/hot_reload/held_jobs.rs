use super::local_jobs::{job_tool, job_wake, open_job_room, ready_codex};
use super::*;
use chariox_relay::{auth::RelayAuthVerifier, config::RelayConfig, server::RelayServer};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};

struct Cloud(Child);
impl Drop for Cloud {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct HeldDirectory(PathBuf);

impl Drop for HeldDirectory {
    fn drop(&mut self) {
        if std::env::var_os("CHARIOX_HOT_RELOAD_KEEP").is_none() {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[tokio::test]
async fn a_held_job_resumes_its_acknowledged_output_after_home_reload() {
    let suite =
        HeldDirectory(std::env::temp_dir().join(format!("hr-held-{:x}", rand::random::<u64>())));
    let worker_root = suite.0.join("worker");
    let home_root = suite.0.join("home");
    let worker_home = worker_root.join("home");
    std::fs::create_dir_all(&worker_home).unwrap();
    let relay_port = free_port();
    let relay_secret = format!("{:032x}", rand::random::<u128>());
    let relay = RelayServer::with_auth_verifier(
        RelayConfig {
            host: "127.0.0.1".into(),
            port: relay_port,
            shared_token: None,
        },
        RelayAuthVerifier::scoped_hmac(
            BTreeMap::from([("held-job-reload".into(), relay_secret.clone())]),
            None,
        ),
    );
    let incoming = relay.bind_listener().await.unwrap();
    let (stop, stop_relay) = tokio::sync::oneshot::channel();
    let relay = tokio::spawn(async move {
        relay
            .run_listener_until(incoming, async {
                let _ = stop_relay.await;
            })
            .await
            .unwrap();
    });

    let mut cloud = Cloud(
        Command::new("node")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/hot_reload/held_job_cloud.mjs"))
            .env("HELD_JOB_RELAY_PORT", relay_port.to_string())
            .env("HELD_JOB_TEST_RELAY_SECRET", relay_secret)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut profile = String::new();
    BufReader::new(cloud.0.stdout.take().unwrap())
        .read_line(&mut profile)
        .unwrap();
    let profile: Value = serde_json::from_str(&profile).expect("Cloud fixture started");

    let worker = start_laptop_kernel(
        &worker_root,
        &[(
            "CHARIOX_CLOUD_RELAY_CONFIG_JSON",
            PathBuf::from(profile["worker"].to_string()),
        )],
    );
    let mut worker_socket = connect(&worker, Duration::from_secs(30)).await;
    wait_for_relay(&mut worker_socket).await;

    let record = home_root.join("record");
    let mut env = ready_codex(&home_root, &record);
    env.push((
        "CHARIOX_CLOUD_RELAY_CONFIG_JSON",
        PathBuf::from(profile["home"].to_string()),
    ));
    let home = start_laptop_kernel(&home_root, &env);
    held_job_drill(&home, &record, &worker_home, &mut worker_socket).await;

    drop(home);
    drop(worker);
    drop(cloud);
    let _ = stop.send(());
    relay.await.unwrap();
}

async fn wait_for_relay(socket: &mut Socket) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = request(socket, json!({"RelayStatus": null})).await;
        if status["RelayStatus"]["status"]["connected"] == true {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "kernel did not connect to the fixture relay"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn held_job_drill(
    home: &Kernel,
    record: &Path,
    worker_home: &Path,
    worker_socket: &mut Socket,
) {
    let mut socket = connect(home, Duration::from_secs(30)).await;
    wait_for_relay(&mut socket).await;
    let project = home.scratch.join("project");
    let (room, task) = open_job_room(&mut socket, &project, record).await;
    let started = job_tool(record, "held-start", "chariox.jobs.run", json!({
        "task_id": task, "origin_prompt_id": task, "machine": "held-job-machine", "timeout_s": 90, "wait_s": 1,
        "outputs": ["result.txt"],
        "command": ["/bin/sh", "-c", "printf 'before\\npartial-'; while [ ! -f finish ]; do sleep 0.05; done; printf 'line\\nafter\\n'; printf artifact > result.txt; exit 7"],
    })).await;
    assert_eq!(started["placement"], "remote", "{started}");
    let job = started["job_id"].as_str().unwrap();
    let args = json!({"task_id": task, "origin_prompt_id": task, "job_id": job});
    let before = request(
        &mut socket,
        json!({"GetSessionState": {"session_id": room}}),
    )
    .await;
    let wake = job_wake(&before, job);
    assert_eq!(wake["fire_count"], 0, "{wake}");
    let wake_id = wake["id"].as_str().unwrap();
    worker_has_acknowledged_output(worker_socket, job).await;

    std::fs::write(record.join("swap"), "").unwrap();
    unsafe { libc::kill(home.child.id() as i32, libc::SIGUSR2) };
    closed_by_the_old_program(socket).await;
    socket = connect(home, Duration::from_secs(30)).await;
    wait_for_relay(&mut socket).await;
    let workspace = worker_home.join("chariox-jobs").join(job).join("workspace");
    std::fs::write(workspace.join("finish"), "").unwrap();

    let mut finished = Value::Null;
    for poll in 0..60 {
        finished = job_tool(
            record,
            &format!("held-status-{poll}"),
            "chariox.jobs.status",
            args.clone(),
        )
        .await;
        if finished["state"] == "finished" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        finished["state"],
        "finished",
        "worker completed its file: {}, job: {finished}",
        workspace.join("result.txt").exists()
    );
    assert_eq!(finished["result"]["exit_code"], 7, "{finished}");
    assert_eq!(
        finished["result"]["output_tail"], "before\npartial-line\nafter\n",
        "{finished}"
    );
    assert_eq!(
        std::fs::read_to_string(finished["result"]["log"].as_str().unwrap()).unwrap(),
        "before\npartial-line\nafter\n",
    );
    assert_eq!(
        finished["result"]["files_back"],
        json!(["result.txt"]),
        "{finished}"
    );
    assert_eq!(
        std::fs::read(project.join("result.txt")).unwrap(),
        b"artifact"
    );
    let after = request(
        &mut socket,
        json!({"GetSessionState": {"session_id": room}}),
    )
    .await;
    let wake = job_wake(&after, job);
    assert_eq!(wake["id"], wake_id, "{wake}");
    assert_eq!(wake["fire_count"], 1, "{wake}");
    let obligations = after["SessionState"]["session"]["agent_tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["task_id"] == task)
        .unwrap()["obligations"]
        .as_array()
        .unwrap();
    assert_eq!(obligations.len(), 1);
    assert_eq!(obligations[0]["status"], "failed");
    std::fs::write(record.join("go"), "").unwrap();
}

async fn worker_has_acknowledged_output(socket: &mut Socket, job: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let directory = format!("/{job}/workspace");
    loop {
        let rooms = request(socket, json!({"ListSessions": null})).await;
        let rooms = rooms["SessionsListed"]["sessions"].as_array().unwrap();
        if let Some(room) = rooms.iter().find(|room| {
            room["worktree_id"]
                .as_str()
                .is_some_and(|root| root.ends_with(&directory))
        }) {
            let session_id = &room["id"];
            let list = request(
                socket,
                json!({"RoomTerminal": {
                    "session_id": session_id, "operation": {"operation": "exec_list"},
                }}),
            )
            .await;
            if let Some(exec_id) = list["RoomTerminal"]["result"]["execs"][0]["exec_id"].as_str() {
                let read = request_result(socket, json!({"RoomTerminal": {
                    "session_id": session_id, "operation": {"operation":"exec_read", "exec_id":exec_id, "after_cursor":0},
                }})).await;
                if read.is_err_and(|error| {
                    error["message"].as_str().is_some_and(|message| {
                        message.contains("resume from the last consumed cursor")
                    })
                }) {
                    return;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "the home kernel never acknowledged the worker output"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
