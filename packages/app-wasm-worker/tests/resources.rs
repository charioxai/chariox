use chariox_app_runtime::wire::{Channel, Message, Sender};
use chariox_app_wasm_worker::{
    ipc::{wall_ms, Ipc},
    limits::Memory,
    scope::Scope,
};
use serde_json::json;
use std::time::Duration;
use tokio::{net::UnixStream, time::Instant};

#[test]
fn configured_guest_memory_leaves_native_runtime_headroom() {
    use chariox_app_wasm_worker::limits::Limits;
    let mut limits = Limits {
        memory_bytes: 128 << 20,
        ..Limits::default()
    };
    limits.validate().unwrap();
    for bytes in [(128 << 20) + 1, 512 << 20, 1_usize << 32] {
        limits.memory_bytes = bytes;
        assert!(limits.validate().is_err(), "admitted {bytes} guest bytes");
    }
}

#[test]
fn incremental_guest_memory_growth_reuses_reserved_headroom() {
    let engine = chariox_app_wasm_worker::engine::engine().unwrap();
    let mut store = wasmtime::Store::new(&engine, ());
    let memory = wasmtime::Memory::new(&mut store, wasmtime::MemoryType::new(1, None)).unwrap();
    memory.grow(&mut store, 1).unwrap();
    memory.write(&mut store, 0, b"preserved").unwrap();
    let pointer = memory.data_ptr(&store);
    for _ in 0..32 {
        memory.grow(&mut store, 1).unwrap();
        assert_eq!(memory.data_ptr(&store), pointer, "copied on a small grow");
    }
    assert_eq!(&memory.data(&store)[..9], b"preserved");
}

#[tokio::test]
async fn cancelled_queued_requests_never_reach_the_broker() {
    use std::os::fd::AsRawFd;
    for drop_import in [false, true] {
        let (worker, supervisor) = UnixStream::pair().unwrap();
        let buffer: libc::c_int = 4096;
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    worker.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_SNDBUF,
                    (&buffer as *const libc::c_int).cast(),
                    std::mem::size_of_val(&buffer) as libc::socklen_t,
                )
            },
            0
        );
        let transport = Ipc::start(worker, "1".into()).unwrap();
        transport
            .ipc
            .respond("blocked".into(), Ok(json!("x".repeat(256 * 1024))))
            .await
            .unwrap();
        // Some bytes arrived, but this frame cannot fit in the socket buffer.
        supervisor.readable().await.unwrap();
        let scope = Scope::new("queued".into(), Instant::now() + Duration::from_secs(5));
        let ipc = transport.ipc.clone();
        let calling = scope.clone();
        let task = tokio::spawn(async move {
            ipc.call(&calling, "state.transaction", json!({"writes":[]}))
                .await
        });
        while transport.ipc.pending_count() == 0 {
            tokio::task::yield_now().await;
        }
        if drop_import {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            scope.cancel("CANCELLED");
            assert_eq!(task.await.unwrap().unwrap_err().code, "CANCELLED");
        }
        assert_eq!(transport.ipc.pending_count(), 0);
        transport
            .ipc
            .respond("after".into(), Ok(json!(null)))
            .await
            .unwrap();
        let mut channel = Channel::new(supervisor, "1".into(), Sender::Worker).unwrap();
        assert!(
            matches!(channel.receive(Duration::from_secs(1)).await.unwrap(), Message::Response { id, .. } if id == "blocked")
        );
        assert!(
            matches!(channel.receive(Duration::from_secs(1)).await.unwrap(), Message::Response { id, .. } if id == "after")
        );
    }
}

#[tokio::test]
async fn cancellation_releases_pending_broker_request_and_sends_cancel() {
    let (worker, supervisor) = UnixStream::pair().unwrap();
    let transport = Ipc::start(worker, "1".into()).unwrap();
    let mut channel = Channel::new(supervisor, "1".into(), Sender::Worker).unwrap();
    let scope = Scope::new("call-1".into(), Instant::now() + Duration::from_secs(5));
    let ipc = transport.ipc.clone();
    let calling = scope.clone();
    let task =
        tokio::spawn(async move { ipc.call(&calling, "state.get", json!({"key":"hold"})).await });
    let Message::Request { id, .. } = channel.receive(Duration::from_secs(1)).await.unwrap() else {
        panic!("expected broker request")
    };
    scope.cancel("CANCELLED");
    assert_eq!(task.await.unwrap().unwrap_err().code, "CANCELLED");
    assert_eq!(transport.ipc.pending_count(), 0);
    let Message::Cancel { id: cancelled, .. } =
        channel.receive(Duration::from_secs(1)).await.unwrap()
    else {
        panic!("expected cancellation")
    };
    assert_eq!(id, cancelled);
}

#[tokio::test]
async fn dropping_an_import_cancels_its_broker_request() {
    let (worker, supervisor) = UnixStream::pair().unwrap();
    let transport = Ipc::start(worker, "1".into()).unwrap();
    let mut channel = Channel::new(supervisor, "1".into(), Sender::Worker).unwrap();
    let ipc = transport.ipc.clone();
    let task = tokio::spawn(async move {
        ipc.call(
            &Scope::new("call-1".into(), Instant::now() + Duration::from_secs(5)),
            "state.get",
            json!({"key":"hold"}),
        )
        .await
    });
    let Message::Request { id, .. } = channel.receive(Duration::from_secs(1)).await.unwrap() else {
        panic!("expected request")
    };
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let Message::Cancel { id: cancelled, .. } =
        channel.receive(Duration::from_secs(1)).await.unwrap()
    else {
        panic!("expected cancel")
    };
    assert_eq!(id, cancelled);
    assert_eq!(transport.ipc.pending_count(), 0);
}

#[tokio::test]
async fn broker_deadline_releases_resources() {
    let (worker, supervisor) = UnixStream::pair().unwrap();
    let transport = Ipc::start(worker, "1".into()).unwrap();
    let mut channel = Channel::new(supervisor, "1".into(), Sender::Worker).unwrap();
    let ipc = transport.ipc.clone();
    let task = tokio::spawn(async move {
        ipc.call(
            &Scope::new("call-1".into(), Instant::now() + Duration::from_millis(30)),
            "state.get",
            json!({"key":"hold"}),
        )
        .await
    });
    assert!(
        matches!(channel.receive(Duration::from_secs(1)).await.unwrap(),Message::Request { deadline_ms,.. } if deadline_ms<=wall_ms()+31)
    );
    assert_eq!(task.await.unwrap().unwrap_err().code, "DEADLINE_EXCEEDED");
    assert!(matches!(
        channel.receive(Duration::from_secs(1)).await.unwrap(),
        Message::Cancel { .. }
    ));
    assert_eq!(transport.ipc.pending_count(), 0);
}

#[test]
fn aggregate_memory_limit_traps_growth_across_memories() {
    let engine = chariox_app_wasm_worker::engine::engine().unwrap();
    let module = wasmtime::Module::new(
        &engine,
        wat::parse_str(
            r#"(module
        (memory 1) (memory 1)
        (func (export "grow") (result i32) i32.const 1 memory.grow 1)
    )"#,
        )
        .unwrap(),
    )
    .unwrap();
    let mut store = wasmtime::Store::new(&engine, Memory::new(2 * 65536));
    store.limiter(|memory| memory);
    store.set_epoch_deadline(1_000_000);
    let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
    let grow = instance
        .get_typed_func::<(), i32>(&mut store, "grow")
        .unwrap();
    let error = grow.call(&mut store, ()).unwrap_err();
    assert!(format!("{error:#}").contains("memory limit"));
}

#[tokio::test]
async fn ordinary_completion_closes_resources_without_signalling_guest_cancel() {
    let scope = Scope::new("finished".into(), Instant::now() + Duration::from_secs(5));
    scope.finish();
    assert_eq!(scope.cancelled().await, "APP_CALL_FINISHED");
    assert!(
        tokio::time::timeout(Duration::from_millis(20), scope.guest_cancelled())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn an_oversized_broker_request_does_not_close_the_worker_channel() {
    let (worker, supervisor) = UnixStream::pair().unwrap();
    let transport = Ipc::start(worker, "1".into()).unwrap();
    let mut channel = Channel::new(supervisor, "1".into(), Sender::Worker).unwrap();
    let scope = Scope::new("call".into(), Instant::now() + Duration::from_secs(5));
    let failure = transport
        .ipc
        .call(&scope, "state.get", json!({"key":"x".repeat(2*1024*1024)}))
        .await
        .unwrap_err();
    assert_eq!(failure.code, "INVALID_ARGUMENT");
    assert_eq!(transport.ipc.pending_count(), 0);
    let ipc = transport.ipc.clone();
    let call =
        tokio::spawn(async move { ipc.call(&scope, "state.get", json!({"key":"ok"})).await });
    let Message::Request { id, .. } = channel.receive(Duration::from_secs(1)).await.unwrap() else {
        panic!("valid broker request expected")
    };
    channel
        .send(
            &Message::Response {
                version: 1,
                generation: "1".into(),
                id,
                outcome: chariox_app_runtime::wire::Outcome::Success(
                    chariox_app_runtime::wire::Success {
                        result: serde_json::Value::Null,
                    },
                ),
            },
            Duration::from_secs(1),
        )
        .await
        .unwrap();
    assert!(call.await.unwrap().unwrap().is_null());
}
