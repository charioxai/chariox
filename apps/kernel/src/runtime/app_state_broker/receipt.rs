//! The SDK receives an opaque receipt and delivery state, never the retained
//! payload, workflow target, attempt counters, or another operation's metadata.

use chariox_app_runtime::app_outbox::{Receipt, ReceiptState};
use serde_json::{json, Value};

pub(super) fn value(receipt: &Receipt) -> Value {
    let state = match receipt.state {
        ReceiptState::Accepted => "accepted",
        ReceiptState::Queued => "queued",
        ReceiptState::Delivered => "delivered",
        ReceiptState::Retryable => "retryable",
        ReceiptState::Failed => "failed",
        ReceiptState::Expired => "expired",
    };
    json!({"receiptId":receipt.receipt_id,"state":state})
}
