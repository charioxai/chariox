//! Durable explicit publisher enrollment decisions on the existing writer.
//! An input key is review material, never an enrollment authorization.
mod api;
mod apply;
mod model;
mod store;
#[cfg(test)]
mod tests;

use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::runtime::app_operation_budget::AppOperationBudget;
pub(crate) use model::{
    PublisherApprovalChallenge, PublisherEnrollmentInput, PublisherOperation,
    PublisherOperationError, PublisherOperationPhase, PublisherReview,
};
use rusqlite::Connection;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use tokio::time::Instant;

type Result<T> = std::result::Result<T, PublisherOperationError>;
enum Command {
    Begin {
        owner: String,
        request: String,
        input: PublisherEnrollmentInput,
        budget: AppOperationBudget,
    },
    Arm {
        owner: String,
        request: String,
        interaction: String,
        deadline: Instant,
        budget: AppOperationBudget,
    },
    Decide {
        challenge: Arc<PublisherApprovalChallenge>,
        accepted: bool,
        budget: AppOperationBudget,
    },
    Cancel {
        owner: String,
        request: String,
        budget: AppOperationBudget,
    },
    Status {
        owner: String,
        request: String,
        budget: AppOperationBudget,
    },
}
enum Reply {
    Operation(PublisherOperation),
    Review(PublisherReview),
}
pub(super) struct PublisherOperationRequest {
    command: Command,
    response: mpsc::Sender<Result<Reply>>,
}
impl std::fmt::Debug for PublisherOperationRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublisherOperationRequest")
            .finish_non_exhaustive()
    }
}
pub(super) fn initialize(
    connection: &Connection,
) -> std::result::Result<(), crate::error::DaemonError> {
    store::initialize(connection).map_err(|_| crate::error::DaemonError::LocalTransport {
        operation: "durable_state.publisher_enrollment_operations",
        message: "Publisher enrollment operation schema could not be initialized".into(),
    })
}
pub(super) fn execute(
    connection: &mut Connection,
    request: PublisherOperationRequest,
    writer_fatal: &AtomicBool,
) -> super::app_event_delivery::WriterDisposition {
    respond(
        request.response,
        apply::execute(connection, request.command),
        writer_fatal,
    )
}
fn respond(
    response: mpsc::Sender<Result<Reply>>,
    result: Result<Reply>,
    writer_fatal: &AtomicBool,
) -> super::app_event_delivery::WriterDisposition {
    let disposition = if matches!(&result, Err(PublisherOperationError::CommitUnknown)) {
        // Fence snapshot readers before the uncertain reply becomes observable,
        // including the interval before the dispatcher consumes Stop.
        writer_fatal.store(true, Ordering::Release);
        super::app_event_delivery::WriterDisposition::Stop
    } else {
        super::app_event_delivery::WriterDisposition::Continue
    };
    let _ = response.send(result);
    disposition
}
