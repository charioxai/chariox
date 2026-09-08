//! Kernel integration against the durable writer and a real fixed-libc worker.
//! The fixture supplies only its own private directory; no path/capability is
//! fabricated. Fault cases use a duplex peer to control cancellation precisely.
use super::*;
use support::*;

mod authority;
mod cancellation;
mod outcomes;
mod support;

#[test]
fn actual_worker_channel_routes_state_and_files_and_preserves_bytes_written_shape() {
    let mut fixture = Fixture::new(Mode::Files);
    fixture.wait_event("worker.fixture.files_complete");
    assert_eq!(
        fixture.observed.private_file().unwrap(),
        Some(b"\0\xfffile".to_vec())
    );
    assert_eq!(fixture.observed.pending_file_replacements().unwrap(), 0);
    fixture.shutdown();
    assert!(fixture.observed.was_reaped());
    assert!(fixture.observed.lease_was_dropped());
}

#[test]
fn strict_wire_decoder_rejects_authority_fields_bad_base64_and_escaping_paths() {
    let fixture = Fixture::new(Mode::Ready);
    fixture.runtime.block_on(async {
        let mut peer = TestPeer::start(fixture.service());
        for (index, params) in [
            serde_json::json!({"path":"fixture-file","contentsBase64":"eA==","owner":"bob"}),
            serde_json::json!({"path":"fixture-file","contentsBase64":"eA==","generation":2}),
            serde_json::json!({"path":"fixture-file","contentsBase64":null}),
            serde_json::json!({"path":"fixture-file","contentsBase64":"%%%"}),
            serde_json::json!({"path":"../fixture-file","contentsBase64":"eA=="}),
            serde_json::json!({"path":"/fixture-file","contentsBase64":"eA=="}),
        ]
        .into_iter()
        .enumerate()
        {
            peer.send(&format!("invalid-{index}"), "files.atomic_replace", params)
                .await;
            let failure = peer.response().await.1.unwrap_err();
            assert_eq!(failure.code, "INVALID_ARGUMENT");
            assert_eq!(failure.retryable, Some(false));
        }
        assert_eq!(fixture.observed.private_file().unwrap(), None);
        assert_eq!(fixture.observed.pending_file_replacements().unwrap(), 0);
        // Empty files are valid and the result is bytesWritten, never a state revision.
        peer.replace("empty", b"").await;
        assert_eq!(
            peer.response().await.1.unwrap(),
            serde_json::json!({"bytesWritten":0})
        );
        peer.close().await;
    });
    assert_eq!(fixture.observed.private_file().unwrap(), Some(Vec::new()));
}
