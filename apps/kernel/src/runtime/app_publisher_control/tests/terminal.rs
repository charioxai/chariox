use super::*;
use crate::{local::*, runtime::command::KernelCommand};
use base64::{engine::general_purpose::STANDARD, Engine as _};

async fn send(f: &Fixture, owner: &str, request: LocalDaemonRequest) -> LocalDaemonResponse {
    let mut command = KernelCommand::from_local_request("publisher-wire", None, None, &request);
    command.caller.user_id = Some(owner.into());
    f.control()
        .execute(&f.state, &command, &request)
        .await
        .unwrap()
}
fn status(response: LocalDaemonResponse) -> AppPublisherEnrollmentSummary {
    match response {
        LocalDaemonResponse::AppPublisherEnrollmentStatus { operation } => operation,
        response => panic!("unexpected publisher response: {response:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_terminal_reviews_a_key_without_granting_it_or_exposing_other_owners() {
    let f = Fixture::new();
    let input = f.input();
    let begin = BeginAppPublisherEnrollmentRequest {
        session_id: input.session_id,
        request_id: "review".into(),
        publisher_id: input.publisher_id,
        key_id: input.key_id,
        public_key_base64: STANDARD.encode(input.public_key),
        expected_revision: "0".into(),
    };
    let denied = send(
        &f,
        "bob",
        LocalDaemonRequest::BeginAppPublisherEnrollment(begin.clone()),
    )
    .await;
    assert!(matches!(
        denied,
        LocalDaemonResponse::AppRequestFailed {
            code: AppRequestErrorCode::Unauthorized
        }
    ));
    for revision in [
        "00",
        "+1",
        "-1",
        "9223372036854775808",
        "18446744073709551616",
    ] {
        let mut bad = begin.clone();
        bad.expected_revision = revision.into();
        assert!(matches!(
            send(
                &f,
                "alice",
                LocalDaemonRequest::BeginAppPublisherEnrollment(bad)
            )
            .await,
            LocalDaemonResponse::AppRequestFailed {
                code: AppRequestErrorCode::InvalidRequest
            }
        ));
    }
    let mut bad = begin.clone();
    bad.public_key_base64.push('=');
    assert!(matches!(
        send(
            &f,
            "alice",
            LocalDaemonRequest::BeginAppPublisherEnrollment(bad)
        )
        .await,
        LocalDaemonResponse::AppRequestFailed {
            code: AppRequestErrorCode::InvalidRequest
        }
    ));
    let pending = status(
        send(
            &f,
            "alice",
            LocalDaemonRequest::BeginAppPublisherEnrollment(begin.clone()),
        )
        .await,
    );
    assert_eq!(pending.phase, AppPublisherEnrollmentPhase::Pending);
    assert!(pending.approved_revision.is_none());
    assert_eq!(
        pending,
        status(
            send(
                &f,
                "alice",
                LocalDaemonRequest::BeginAppPublisherEnrollment(begin)
            )
            .await
        )
    );
    assert!(f
        .control()
        .0
        .shared
        .store
        .list_app_publishers("alice")
        .unwrap()
        .is_empty());
    let request = AppPublisherEnrollmentRequest {
        request_id: "review".into(),
    };
    for other in [
        LocalDaemonRequest::GetAppPublisherEnrollment(request.clone()),
        LocalDaemonRequest::CancelAppPublisherEnrollment(request.clone()),
    ] {
        assert!(matches!(
            send(&f, "bob", other).await,
            LocalDaemonResponse::AppRequestFailed {
                code: AppRequestErrorCode::NotFound
            }
        ));
    }
    let cancelled = status(
        send(
            &f,
            "alice",
            LocalDaemonRequest::CancelAppPublisherEnrollment(request.clone()),
        )
        .await,
    );
    assert_eq!(cancelled.phase, AppPublisherEnrollmentPhase::Cancelled);
    assert_eq!(
        cancelled,
        status(
            send(
                &f,
                "alice",
                LocalDaemonRequest::GetAppPublisherEnrollment(request)
            )
            .await
        )
    );
    assert!(f
        .control()
        .0
        .shared
        .store
        .list_app_publishers("alice")
        .unwrap()
        .is_empty());
    f.shutdown().await;
}
