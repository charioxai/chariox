//! Small fixed-libc/real-peer seam regression, independent of the kernel build.
//! The delegate below is a test harness, not a production activation authority.
#![cfg(all(
    feature = "test-fixtures",
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]

use chariox_app_package::{pack, verify, Limits, Manifest, TrustedPublisher, VerificationPolicy};
use chariox_app_runtime::{
    wire::RemoteError,
    worker_peer::{Broker, BrokerFuture, BrokerRequest, PeerLimits, WorkerPeer},
    worker_process::test_fixture::{Fixture, Mode},
};
use ed25519_dalek::SigningKey;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

struct ReadyDelegate(Arc<AtomicUsize>);
impl Broker for ReadyDelegate {
    fn handle(&self, request: BrokerRequest) -> BrokerFuture {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            match request.method.as_str() {
                "state.get" => Err(RemoteError {
                    code: "APP_NOT_READY".into(),
                    message: "App worker is not available".into(),
                    retryable: Some(false),
                }),
                "worker.ready" => {
                    assert_eq!(
                        request.params,
                        json!({"tools":[],"events":[],"lifecycle":[]})
                    );
                    Ok(Value::Null)
                }
                _ => panic!("unexpected fixed fixture request"),
            }
        })
    }
}

#[test]
fn fixed_native_sdk_handshake_uses_the_real_peer_control_event_namespace() {
    exercise("installed");
}

#[test]
fn generated_first_install_identity_reaches_ready_on_the_real_native_peer() {
    exercise("app_0123456789abcdef0123456789abcdef");
}

fn exercise(installation: &str) {
    let native = Fixture::compile().unwrap();
    let manifest: Manifest = serde_json::from_value(json!({
        "schema":"chariox.app.v1","appId":"com.example.fixture","version":"1.0.0",
        "publisher":{"id":"com.example","keyId":"fixture","name":"Fixture"},
        "sdkVersion":chariox_app_package::SUPPORTED_SDK_VERSION,"appContractVersion":1,"minKernelProtocol":500,
        "resourcePolicy":"chariox.app.resources.v1","runtime":{"engine":"node","entry":"runtime/main.js"},
        "ui":{"entry":"ui/index.html"},"capabilities":{}
    })).unwrap();
    let key = SigningKey::from_bytes(&[63; 32]);
    let bytes = pack(
        &manifest,
        &BTreeMap::from([
            (
                "runtime/main.js".into(),
                b"export default function register() {}".to_vec(),
            ),
            (
                "ui/index.html".into(),
                b"<!doctype html><title>Fixture</title>".to_vec(),
            ),
        ]),
        &key,
        &Limits::default(),
    )
    .unwrap();
    let package = verify(
        &bytes,
        &VerificationPolicy::new(
            500,
            vec![TrustedPublisher {
                publisher_id: "com.example".into(),
                key_id: "fixture".into(),
                public_key: key.verifying_key(),
            }],
        ),
    )
    .unwrap();
    let (mut process, observed) = native
        .spawn_for_installation_blocking(Mode::Ready, &package, installation)
        .unwrap();
    assert_eq!(process.installation_id(), installation);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let result = runtime.block_on(async {
        let channel = process.take_sdk_channel().unwrap();
        let (peer, mut events, task) = WorkerPeer::start(
            channel,
            Arc::new(ReadyDelegate(calls.clone())),
            PeerLimits::default(),
        )
        .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(3), async {
            let first = events
                .recv()
                .await
                .ok_or("peer closed before first control event")?;
            assert_eq!(first.name, "worker.fixture.before_ready_rejected");
            let second = events.recv().await.ok_or("peer closed before ready ACK")?;
            assert_eq!(second.name, "worker.fixture.ready_ack");
            Ok::<(), &'static str>(())
        })
        .await;
        peer.close();
        let peer_result = tokio::time::timeout(Duration::from_secs(3), task.join()).await;
        (result, peer_result)
    });
    let exit = process.shutdown_blocking().unwrap();
    assert!(
        result.0.is_ok_and(|inner| inner.is_ok()) && matches!(result.1, Ok(Ok(()))),
        "fixed SDK handshake failed; peer={:?}, native exit code={:?}, signal={:?}",
        result.1,
        exit.code,
        exit.signal
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(observed.ready_was_acknowledged());
    assert!(observed.was_reaped());
    assert!(observed.lease_was_dropped());
}
