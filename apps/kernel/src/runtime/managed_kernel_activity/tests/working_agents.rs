//! MP-08 / MP-09 / MP-10 / MP-11: the signed activity report carries the
//! number of working agents.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentWake, Operation};

#[test]
fn managed_working_agents_signed_report_carries_the_count() {
    let at = "1970-01-01T00:00:01.000Z";
    let payload = signed_activity_value(&binding(), 7, 3, at).unwrap();
    assert_eq!(payload["workingAgentCount"], 3);
    let mut worker = binding();
    worker.worker = true;
    let payload = signed_activity_value(&worker, 7, 3, at).unwrap();
    assert_eq!(payload["workingAgentCount"], 3);

    let signature =
        |working_agent_count| activity_signature(&binding(), 7, working_agent_count, at).unwrap();
    assert_ne!(
        signature(3),
        signature(2),
        "the working count is covered by the signature"
    );
}

#[test]
fn managed_working_agents_change_alone_is_reported_again() {
    let mut cursor = ActivityCursor::default();
    let idle = crate::runtime::state::ManagedActivityObservation {
        working_agent_count: 0,
        changed_at_ms: 1_000,
    };
    let first = cursor.next_report(idle).unwrap().expect("first report");
    cursor
        .accept_response(ReportActivityResponse {
            accepted_sequence: first.sequence,
            working_agent_count: 0,
        })
        .unwrap();
    assert_eq!(cursor.next_report(idle).unwrap(), None);

    let waiting = crate::runtime::state::ManagedActivityObservation {
        working_agent_count: 1,
        changed_at_ms: 2_000,
    };
    let second = cursor
        .next_report(waiting)
        .unwrap()
        .expect("a registered wake must be reported although nothing is running");
    assert_eq!(second.sequence, 2);
    assert_eq!(second.working_agent_count, 1);
}

fn arm_timer(store: &crate::durable_state::DurableKernelStateStore, task: &str) {
    let wake = AgentWake {
        id: "timer-1".into(),
        task_id: task.into(),
        room_id: "session-1".into(),
        agent_id: "agent-1".into(),
        registration_id: "completion-timer-1".into(),
        kind: "timer".into(),
        label: "check back".into(),
        state: String::new(),
        created_at_ms: 10,
        verified_at_ms: None,
        next_due_ms: Some(crate::session::unix_epoch_ms() + 3_600_000),
        interval_ms: None,
        command: vec![],
        match_text: None,
        matched_at_ms: None,
        pid: None,
        exit_code: None,
        summary: String::new(),
        detail: serde_json::Value::Null,
        fire_count: 0,
        missed_fires: 0,
        last_fired_at_ms: None,
        last_sequence: None,
        last_delivery: None,
        last_delivered_at_ms: None,
        last_acknowledged_at_ms: None,
        alerted_sequence: None,
    };
    store
        .agent_lifecycle(Operation::CreateWake {
            task: task.into(),
            prompt: task.into(),
            wake,
        })
        .expect("timer should arm");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn managed_working_agents_report_reaches_cloud_signed() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind activity fixture");
    let address = listener.local_addr().expect("activity fixture address");
    let (bodies_tx, bodies_rx) = std::sync::mpsc::channel();
    let fixture = std::thread::spawn(move || {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().expect("accept activity report");
            let body = http_request_body(&read_http_request(&mut stream));
            let response = serde_json::json!({
                "acceptedSequence": body["sequence"],
                "workingAgentCount": body["workingAgentCount"],
            });
            write_http_response(&mut stream, &response);
            bodies_tx.send(body).expect("publish activity report");
        }
    });

    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let runtime = CommandRouter::with_interactive_capacity(app, 1).runtime_state();
    runtime
        .ensure_managed_activity_tracking("kernel-1")
        .expect("activate test tracking before activity");
    runtime.restore_room_for_managed_activity_test("session-1");
    let store = runtime.durable_state_store_for_managed_activity_test();
    let task = "prompt-1".to_string();
    store
        .agent_lifecycle(Operation::Begin {
            owner: "owner".into(),
            room: "session-1".into(),
            agent: "agent-1".into(),
            prompt: task.clone(),
            run: Some("run-1".into()),
            now: 1,
        })
        .expect("task should begin");

    let mut activity_binding = binding();
    activity_binding.api_url = format!("http://{address}");
    let reporter = ManagedKernelActivityReporter {
        binding: activity_binding,
        confirmation_wait_started: None,
        persistence_retry_started: None,
    };
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let reporter_runtime = runtime.clone();
    let reporter_task =
        tokio::spawn(async move { reporter.run(reporter_runtime, shutdown_rx).await });
    let next_report = || {
        bodies_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the reporter should post a report")
    };

    let idle = next_report();
    assert_eq!(idle["workingAgentCount"], 0);

    arm_timer(&store, &task);
    let waiting = next_report();
    assert_eq!(waiting["sequence"], 2);
    assert_eq!(waiting["workingAgentCount"], 1);
    let expected = activity_signature(
        &binding(),
        2,
        1,
        waiting["activityChangedAt"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(waiting["signature"], expected);

    store
        .agent_lifecycle(Operation::CancelWake {
            id: "timer-1".into(),
            task: task.clone(),
            prompt: Some(task.clone()),
            pending: false,
        })
        .expect("timer should cancel");
    let settled = next_report();
    assert_eq!(settled["sequence"], 3);
    assert_eq!(settled["workingAgentCount"], 0);

    shutdown_tx.send(true).expect("stop activity reporter");
    tokio::time::timeout(Duration::from_secs(2), reporter_task)
        .await
        .expect("activity reporter should stop")
        .expect("activity reporter task should not panic")
        .expect("activity reporter should succeed");
    fixture.join().expect("activity fixture should stop");
}
