//! Serialize HTTP enqueueing with the existing installation/signer writer.
//! This transaction contains no mutation: no DB commit follows a socket effect.
use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::runtime::app_http::{HttpError, HttpJob};
use rusqlite::{Connection, TransactionBehavior};
impl DurableKernelStateStore {
    pub(crate) fn enqueue_app_http(&self, job: HttpJob) {
        // On a closed writer, dropping the typed job closes its one response.
        // No transport operation has started and no alternate writer is used.
        let _ = self
            .writer
            .enqueue(DurableWriterRequest::AppHttp(Box::new(job)));
    }
}
pub(super) fn execute(connection: &mut Connection, job: HttpJob) {
    match connection.transaction_with_behavior(TransactionBehavior::Immediate) {
        Ok(transaction) => {
            job.submit(&transaction);
            drop(transaction);
        }
        Err(_) => job.reject(HttpError::Busy),
    }
}
