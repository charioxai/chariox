use super::*;
use crate::{ipc::wall_ms, migration::MigrationWorker};
use chariox_app_runtime::wire::{Channel, Outcome, Sender, Success};
use serde_json::Value;

async fn completion_race(cancelled: bool) {
    // Keep real component execution, but remove wall-clock epoch yields so
    // guest completion and the stop are ready in the same supervisor poll.
    let engine = WorkerEngine {
        engine: crate::engine::engine().unwrap(),
        stop: Arc::new(AtomicBool::new(false)),
        ticker: None,
    };
    let root = std::path::PathBuf::from(
        std::env::var("CHARIOX_WORKER_DRILL_DIR").expect("run scripts/wasi-drill.sh"),
    );
    let core = std::fs::read(root.join("migration.core.wasm")).unwrap();
    let bytes = wit_component::ComponentEncoder::default()
        .module(&core)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap();
    let component = engine
        .compile(&bytes, chariox_app_wit::MIGRATION_WORLD)
        .unwrap();
    let worker = MigrationWorker::new(engine, component, Limits::default()).unwrap();
    let (stream, peer) = tokio::net::UnixStream::pair().unwrap();
    let mut transport = Ipc::start(stream, "1".into()).unwrap();
    let mut channel = Channel::new(peer, "1".into(), Sender::Worker).unwrap();
    let scope = Scope::new(
        "completion-race".into(),
        tokio::time::Instant::now() + Duration::from_secs(5),
    );
    let context = crate::migration::types::InvocationContext {
        deadline_ms: wall_ms() + 5000,
        installation_id: Some("migration-result-drill".into()),
        room_id: None,
        operation_id: None,
        agent_id: None,
        task_id: None,
        turn_id: None,
        actor: None,
    };
    let running = worker.run(transport.ipc.clone(), scope.clone(), 4, 5, context);
    tokio::pin!(running);
    let message = tokio::select! {
        biased;
        result = &mut running => panic!("migration finished before final import: {result:?}"),
        message = channel.receive(Duration::from_secs(2)) => message.unwrap(),
    };
    let Message::Request {
        id, method, params, ..
    } = message
    else {
        panic!("final state import expected")
    };
    assert_eq!(method, "state.get");
    assert_eq!(params["key"], "completion-race");
    channel
        .send(
            &Message::Response {
                version: 1,
                generation: "1".into(),
                id,
                outcome: Outcome::Success(Success {
                    result: Value::Null,
                }),
            },
            Duration::from_secs(1),
        )
        .await
        .unwrap();
    // The reader processes frames in order. The barrier proves the final
    // import's reply is ready without polling the migration again.
    channel
        .send(
            &Message::Event {
                version: 1,
                generation: "1".into(),
                name: "barrier".into(),
                data: Value::Null,
            },
            Duration::from_secs(1),
        )
        .await
        .unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), transport.incoming.recv())
            .await
            .unwrap(),
        Some(Message::Event { name, .. }) if name == "barrier"
    ));
    assert_eq!(scope.reason(), None);
    if cancelled {
        scope.cancel("CANCELLED");
    } else {
        tokio::time::pause();
        tokio::time::advance(scope.deadline() - tokio::time::Instant::now()).await;
        assert_eq!(scope.reason(), None);
    }
    let result = tokio::time::timeout(Duration::from_secs(1), running)
        .await
        .unwrap();
    let expected = if cancelled {
        "CANCELLED"
    } else {
        "DEADLINE_EXCEEDED"
    };
    assert_eq!(result.unwrap_err().code, expected);
    assert_eq!(scope.reason(), Some(expected));
}

#[tokio::test]
#[ignore = "source migration guest required; run scripts/wasi-drill.sh"]
async fn cancellation_beats_ready_guest_success() {
    completion_race(true).await;
}

#[tokio::test]
#[ignore = "source migration guest required; run scripts/wasi-drill.sh"]
async fn expiry_beats_ready_guest_success_and_records_reason() {
    completion_race(false).await;
}
