use super::*;
use crate::durable_state::app_files::TestPublicationFault;
use std::sync::atomic::Ordering;

fn uncertain(drop_writer_reply: bool) {
    let fixture = Fixture::new(Mode::Ready);
    let fault = Arc::new(TestPublicationFault {
        drop_reply: drop_writer_reply,
        ..Default::default()
    });
    fixture.runtime.block_on(async {
        let mut service = fixture.service();
        service.publication_fault = Some(fault.clone());
        service.panic_after_publish = !drop_writer_reply;
        let mut peer = TestPeer::start(service);
        peer.replace("uncertain", b"already published").await;
        let (id, result) = peer.response().await;
        assert_eq!(id, "uncertain");
        let failure = result.unwrap_err();
        assert_eq!(failure.code, "APP_FILE_OUTCOME_UNCERTAIN");
        assert_eq!(failure.retryable, Some(false));
        assert!(!failure.message.contains("fixture"));
        assert!(!failure.message.contains("/tmp"));
        // Close and join all broker tasks before asserting the final count. The
        // injected failure follows a real rename+directory sync in the writer.
        peer.close().await;
    });
    assert_eq!(
        fault.publications.load(Ordering::SeqCst),
        1,
        "unknown completion must never cause an automatic second publication"
    );
    assert_eq!(
        fixture.observed.private_file().unwrap(),
        Some(b"already published".to_vec())
    );
    assert_eq!(fixture.observed.pending_file_replacements().unwrap(), 0);
    assert_eq!(fixture.admission.available_permits(), 8);
    // A later, explicit call still works. Lost reply is not a poisoned writer
    // and is not treated as evidence that the earlier filesystem effect failed.
    fixture.runtime.block_on(async {
        let mut peer = TestPeer::start(fixture.service());
        peer.replace("explicit", b"explicit replacement").await;
        assert_eq!(
            peer.response().await.1.unwrap(),
            serde_json::json!({"bytesWritten":20})
        );
        peer.close().await;
    });
}

#[test]
fn publication_with_lost_writer_reply_is_uncertain_and_never_retried() {
    uncertain(true);
}

#[test]
fn publication_with_lost_blocking_task_completion_is_uncertain_and_never_retried() {
    uncertain(false);
}
