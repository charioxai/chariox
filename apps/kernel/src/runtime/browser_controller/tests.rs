mod fixture;
mod hosted;

use super::*;
use fixture::{Mode, Peer};
use serde_json::json;
use std::sync::{atomic::AtomicUsize, Mutex};

fn target() -> BrowserTarget {
    BrowserTarget {
        endpoint: "127.0.0.1:9222".parse().unwrap(),
        target_id: "chosen-target".into(),
        environment_id: "room-environment".into(),
        tab_id: "app-tab".into(),
        runtime_generation: 7,
    }
}

#[tokio::test]
async fn exact_target_required_before_any_accessibility_or_input() {
    let (socket, peer) = Peer::spawn(Mode::WrongTarget).await;
    assert!(matches!(
        BrowserController::start(target(), Arc::new(()), socket).await,
        Err(BrowserError::Protocol)
    ));
    peer.finish().await;
    assert_eq!(*peer.methods.lock().unwrap(), ["Target.getTargetInfo"]);
}

#[tokio::test]
async fn shared_handles_observe_one_target_and_reject_stale_generation() {
    let (socket, peer) = Peer::spawn(Mode::Normal).await;
    let owner = BrowserController::start(target(), Arc::new(()), socket)
        .await
        .unwrap();
    let first = owner.handle();
    let second = first.clone();
    let snapshot = first.snapshot().await.unwrap();
    assert_eq!(
        snapshot.accessibility["nodes"][0]["name"]["value"],
        "Chosen App"
    );
    assert_eq!(
        snapshot.reference,
        second.snapshot().await.unwrap().reference
    );
    let mut stale = snapshot.reference;
    stale.runtime_generation += 1;
    assert_eq!(
        second
            .input(stale, BrowserInput::Text("effect".into()), Arc::new(()))
            .await,
        Err(BrowserError::StaleReference)
    );
    assert_eq!(peer.inputs.load(Ordering::Acquire), 0);
    owner.shutdown().await.unwrap();
    peer.finish().await;
    assert!(!peer
        .methods
        .lock()
        .unwrap()
        .iter()
        .any(|method| method == "Target.closeTarget"));
}

#[tokio::test]
async fn navigation_after_input_is_uncertain_and_never_retried() {
    let (socket, peer) = Peer::spawn(Mode::NavigateOnInput).await;
    let owner = BrowserController::start(target(), Arc::new(()), socket)
        .await
        .unwrap();
    let handle = owner.handle();
    let reference = handle.snapshot().await.unwrap().reference;
    assert_eq!(
        handle
            .input(reference, BrowserInput::Text("once".into()), Arc::new(()))
            .await,
        Err(BrowserError::OutcomeUncertain)
    );
    assert_eq!(peer.inputs.load(Ordering::Acquire), 1);
    assert!(handle.snapshot().await.is_err());
    let _ = owner.shutdown().await;
    peer.finish().await;
}

#[tokio::test]
async fn cancelled_caller_retains_transmitted_input_reservation_until_drain() {
    let (socket, peer) = Peer::spawn(Mode::HoldInput).await;
    let owner = BrowserController::start(target(), Arc::new(()), socket)
        .await
        .unwrap();
    let handle = owner.handle();
    let reference = handle.snapshot().await.unwrap().reference;
    let reservation = Arc::new(());
    let weak = Arc::downgrade(&reservation);
    let caller = tokio::spawn(async move {
        handle
            .input(reference, BrowserInput::Text("once".into()), reservation)
            .await
    });
    peer.wait_for_input().await;
    caller.abort();
    let _ = caller.await;
    assert!(
        weak.upgrade().is_some(),
        "transmitted action still owns admission"
    );
    peer.release.notify_one();
    owner.handle().snapshot().await.unwrap(); // Same actor has drained the input.
    assert!(weak.upgrade().is_none());
    assert_eq!(peer.inputs.load(Ordering::Acquire), 1);
    owner.shutdown().await.unwrap();
    peer.finish().await;
}

#[tokio::test]
async fn bounded_queue_skips_cancelled_commands_without_effects() {
    let (socket, peer) = Peer::spawn(Mode::HoldInput).await;
    let owner = BrowserController::start(target(), Arc::new(()), socket)
        .await
        .unwrap();
    let handle = owner.handle();
    let reference = handle.snapshot().await.unwrap().reference;
    let queued_reference = reference.clone();
    let input_handle = handle.clone();
    let input = tokio::spawn(async move {
        input_handle
            .input(reference, BrowserInput::Text("once".into()), Arc::new(()))
            .await
    });
    peer.wait_for_input().await;
    let mut queued = Vec::new();
    for _ in 0..COMMAND_CAPACITY {
        let (response, receive) = oneshot::channel();
        handle
            .commands
            .try_send(Command {
                operation: Operation::Input(
                    queued_reference.clone(),
                    BrowserInput::Text("cancelled".into()),
                ),
                _reservation: Some(Arc::new(())),
                transmitted: Arc::new(AtomicBool::new(false)),
                deadline: Instant::now() + COMMAND_TIMEOUT,
                response,
            })
            .unwrap_or_else(|_| panic!("bounded queue unexpectedly full"));
        queued.push(receive);
    }
    assert!(matches!(handle.snapshot().await, Err(BrowserError::Busy)));
    drop(queued);
    peer.release.notify_one();
    input.await.unwrap().unwrap();
    handle.snapshot().await.unwrap();
    assert_eq!(peer.inputs.load(Ordering::Acquire), 1);
    assert_eq!(
        peer.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| *m == "Accessibility.getFullAXTree")
            .count(),
        2
    );
    owner.shutdown().await.unwrap();
    peer.finish().await;
}

#[tokio::test]
async fn owner_drop_closes_channel_and_reports_inflight_input_uncertain() {
    let (socket, peer) = Peer::spawn(Mode::HoldInput).await;
    let lease = Arc::new(());
    let weak = Arc::downgrade(&lease);
    let owner = BrowserController::start(target(), lease, socket)
        .await
        .unwrap();
    let handle = owner.handle();
    let reference = handle.snapshot().await.unwrap().reference;
    let input = tokio::spawn(async move {
        handle
            .input(reference, BrowserInput::Text("once".into()), Arc::new(()))
            .await
    });
    peer.wait_for_input().await;
    drop(owner);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), input)
            .await
            .unwrap()
            .unwrap(),
        Err(BrowserError::OutcomeUncertain)
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while weak.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    peer.release.notify_one();
    peer.finish().await;
}

#[tokio::test]
async fn screencast_acknowledges_and_retains_only_latest_frame_then_clears_on_close() {
    let (socket, peer) = Peer::spawn(Mode::Normal).await;
    let owner = BrowserController::start(target(), Arc::new(()), socket)
        .await
        .unwrap();
    let mut frames = owner.handle().frames();
    peer.frame(1).await;
    tokio::time::timeout(Duration::from_secs(2), frames.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frames.borrow_and_update().as_ref().unwrap().jpeg[2], 1);
    peer.frame(2).await;
    tokio::time::timeout(Duration::from_secs(2), frames.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frames.borrow_and_update().as_ref().unwrap().jpeg[2], 2);
    owner.handle().snapshot().await.unwrap(); // Ack writes precede this command.
    assert_eq!(
        peer.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| *m == "Page.screencastFrameAck")
            .count(),
        2
    );
    owner.shutdown().await.unwrap();
    assert!(frames.borrow().is_none());
    peer.finish().await;
}

#[test]
fn hostile_protocol_payloads_and_nonlocal_targets_are_rejected() {
    let mut selected = target();
    selected.endpoint = "192.0.2.1:9222".parse().unwrap();
    assert_eq!(selected.validate(), Err(BrowserError::Invalid));
    let reference = BrowserReference {
        environment_id: "environment".into(),
        tab_id: "tab".into(),
        runtime_generation: 1,
        document_revision: 1,
        controller_epoch: 1,
    };
    let frame = json!({"sessionId":1,"data":"not base64","metadata":{"deviceWidth":800,"deviceHeight":600}});
    assert!(matches!(
        protocol::frame(&frame, reference),
        Err(BrowserError::Protocol)
    ));
    assert!(protocol::accessibility(json!({"nodes":vec![json!({"nodeId":"1"});4097]})).is_err());
    assert!(BrowserInput::Text("x".repeat(4097)).validate().is_err());
    assert!(BrowserInput::Key {
        key: "UnboundedShortcut".into(),
        pressed: true
    }
    .validate()
    .is_err());
}

#[tokio::test]
async fn expired_command_has_no_effect_and_does_not_close_controller() {
    let (socket, peer) = Peer::spawn(Mode::Normal).await;
    let owner = BrowserController::start(target(), Arc::new(()), socket)
        .await
        .unwrap();
    let handle = owner.handle();
    let reference = handle.snapshot().await.unwrap().reference;
    let (response, receive) = oneshot::channel();
    handle
        .commands
        .try_send(Command {
            operation: Operation::Input(reference, BrowserInput::Text("expired".into())),
            _reservation: Some(Arc::new(())),
            transmitted: Arc::new(AtomicBool::new(false)),
            deadline: Instant::now() - Duration::from_millis(1),
            response,
        })
        .unwrap_or_else(|_| panic!("unexpected full command queue"));
    assert!(matches!(
        receive.await.unwrap(),
        Err(BrowserError::Deadline)
    ));
    handle.snapshot().await.unwrap();
    assert_eq!(peer.inputs.load(Ordering::Acquire), 0);
    owner.shutdown().await.unwrap();
    peer.finish().await;
}

#[tokio::test]
async fn reconnect_does_not_reuse_document_references_from_previous_controller() {
    let (socket, first_peer) = Peer::spawn(Mode::Normal).await;
    let first = BrowserController::start(target(), Arc::new(()), socket)
        .await
        .unwrap();
    let old_reference = first.handle().snapshot().await.unwrap().reference;
    first.shutdown().await.unwrap();
    first_peer.finish().await;
    let (socket, second_peer) = Peer::spawn(Mode::Normal).await;
    let second = BrowserController::start(target(), Arc::new(()), socket)
        .await
        .unwrap();
    let current_reference = second.handle().snapshot().await.unwrap().reference;
    assert_eq!(
        old_reference.document_revision,
        current_reference.document_revision
    );
    assert_eq!(
        old_reference.runtime_generation,
        current_reference.runtime_generation
    );
    assert_ne!(
        old_reference.controller_epoch,
        current_reference.controller_epoch
    );
    assert_eq!(
        second
            .handle()
            .input(
                old_reference,
                BrowserInput::Text("stale".into()),
                Arc::new(())
            )
            .await,
        Err(BrowserError::StaleReference)
    );
    assert_eq!(second_peer.inputs.load(Ordering::Acquire), 0);
    second.shutdown().await.unwrap();
    second_peer.finish().await;
}
