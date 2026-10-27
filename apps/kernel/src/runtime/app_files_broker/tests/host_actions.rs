//! Actual BackendBroker dispatch over the worker SDK wire, with the durable
//! writer and verified native-worker fixture. No host effect happens here.
use super::*;
use crate::{durable_state::app_host_actions::HostActionCommand, local::AppHostAction};

#[test]
fn app_host_sdk_dispatch_creates_bounded_pending_offers_and_rejects_non_http_links() {
    let fixture = Fixture::with_clipboard(Mode::Ready, true);
    fixture.runtime.block_on(async {
        let mut peer = TestPeer::start_with(fixture.broker());
        for (method, params) in [
            (
                "host.open_link",
                serde_json::json!({"url":"file:///tmp/no"}),
            ),
            (
                "host.open_link",
                serde_json::json!({"url":"javascript:alert(1)"}),
            ),
            (
                "host.clipboard_write",
                serde_json::json!({"text":"x", "owner":"bob"}),
            ),
        ] {
            peer.send("bad", method, params).await;
            assert_eq!(
                peer.response().await.1.unwrap_err().code,
                "INVALID_ARGUMENT"
            );
        }
        peer.send("oversize", "host.clipboard_write", serde_json::json!({"text":"x".repeat(crate::runtime::app_host_broker::MAX_CLIPBOARD_BYTES + 1)})).await;
        assert_eq!(peer.response().await.1.unwrap_err().code, "LIMIT_EXCEEDED");
        peer.send(
            "copy",
            "host.clipboard_write",
            serde_json::json!({"text":"Hello\n世界"}),
        )
        .await;
        let copy = peer.response().await.1.unwrap();
        assert_eq!(copy["state"], "pending");
        peer.send(
            "link",
            "host.open_link",
            serde_json::json!({"url":"https://example.org/a?x=%20"}),
        )
        .await;
        assert_eq!(peer.response().await.1.unwrap()["state"], "pending");
        let store = fixture.store.clone();
        let operation_id = copy["operationId"].as_str().unwrap().to_owned();
        let accepted = tokio::task::spawn_blocking(move || {
            store.app_host_action(HostActionCommand::Accept {
                owner: "alice".into(),
                operation_id,
                now_ms: crate::session::unix_epoch_ms(),
            })
        })
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        assert_eq!(
            accepted.action,
            AppHostAction::ClipboardWrite {
                text: "Hello\n世界".into()
            }
        );
        peer.close().await;
    });
}

#[test]
fn app_host_clipboard_requires_the_signed_write_capability_but_link_offers_do_not() {
    let fixture = Fixture::new(Mode::Ready);
    fixture.runtime.block_on(async {
        let mut peer = TestPeer::start_with(fixture.broker());
        peer.send(
            "denied",
            "host.clipboard_write",
            serde_json::json!({"text":"no"}),
        )
        .await;
        assert_eq!(
            peer.response().await.1.unwrap_err().code,
            "CAPABILITY_REQUIRED"
        );
        peer.send(
            "link",
            "host.open_link",
            serde_json::json!({"url":"https://example.org"}),
        )
        .await;
        assert_eq!(peer.response().await.1.unwrap()["state"], "pending");
        peer.close().await;
    });
}
