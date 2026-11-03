use super::*;
use chariox_app_runtime::wire::{Channel, Sender};

async fn start() -> (
    MigrationWorker,
    chariox_app_wasm_worker::ipc::Transport,
    Channel<tokio::net::UnixStream>,
) {
    let engine = WorkerEngine::new().unwrap();
    let component = engine
        .compile(
            &support::component("migration"),
            chariox_app_wit::MIGRATION_WORLD,
        )
        .unwrap();
    let worker = MigrationWorker::new(engine, component, Limits::default()).unwrap();
    let (stream, peer) = tokio::net::UnixStream::pair().unwrap();
    let transport = Ipc::start(stream, "1".into()).unwrap();
    (
        worker,
        transport,
        Channel::new(peer, "1".into(), Sender::Worker).unwrap(),
    )
}
fn context() -> InvocationContext {
    InvocationContext {
        deadline_ms: wall_ms() + 5000,
        installation_id: Some("migration-stop-drill".into()),
        room_id: None,
        operation_id: None,
        agent_id: None,
        task_id: None,
        turn_id: None,
        actor: None,
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source migration guest required; run scripts/wasi-drill.sh"]
async fn migration_rejects_precancelled_immediate_success() {
    let (worker, transport, _channel) = start().await;
    let scope = Scope::new(
        "pre-cancelled".into(),
        tokio::time::Instant::now() + Duration::from_secs(5),
    );
    scope.cancel("CANCELLED");
    let result = worker
        .run(transport.ipc.clone(), scope, 0, 1, context())
        .await;
    assert_eq!(result.unwrap_err().code, "CANCELLED");
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source migration guest required; run scripts/wasi-drill.sh"]
async fn migration_rejects_expired_immediate_success() {
    let (worker, transport, _channel) = start().await;
    let scope = Scope::new(
        "expired".into(),
        tokio::time::Instant::now() - Duration::from_secs(1),
    );
    let result = worker
        .run(transport.ipc.clone(), scope, 0, 1, context())
        .await;
    assert_eq!(result.unwrap_err().code, "DEADLINE_EXCEEDED");
}
async fn completion_race(cancelled: bool) {
    let (worker, transport, mut channel) = start().await;
    let scope = Scope::new(
        "race".into(),
        tokio::time::Instant::now() + Duration::from_secs(if cancelled { 5 } else { 1 }),
    );
    let signal = scope.clone();
    let task = tokio::spawn(async move {
        worker
            .run(transport.ipc.clone(), scope, 4, 5, context())
            .await
    });
    let Message::Request { method, params, .. } =
        channel.receive(Duration::from_secs(2)).await.unwrap()
    else {
        panic!("final state import expected")
    };
    assert_eq!(method, "state.get");
    assert_eq!(params["key"], "completion-race");
    if cancelled {
        signal.cancel("CANCELLED");
    }
    let result = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        result.unwrap_err().code,
        if cancelled {
            "CANCELLED"
        } else {
            "DEADLINE_EXCEEDED"
        }
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source migration guest required; run scripts/wasi-drill.sh"]
async fn migration_cancellation_beats_guest_success_after_final_import() {
    completion_race(true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source migration guest required; run scripts/wasi-drill.sh"]
async fn migration_deadline_beats_guest_success_after_final_import() {
    completion_race(false).await;
}
