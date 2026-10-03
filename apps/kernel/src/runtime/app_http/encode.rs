//! The response side of the SDK stream contract, shared by actual operations
//! and the cross-language snapshot. These values contain no connection secret.
use super::{streams::ReadResult, transport::ResponseHead};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

pub(super) fn open(id: &str) -> Value {
    json!({"streamId":id})
}
pub(super) fn written(bytes: usize) -> Value {
    json!({"bytesWritten":bytes})
}
pub(super) fn headers(head: Option<ResponseHead>) -> Value {
    match head {
        None => json!({"pending":true}),
        Some(head) => {
            json!({"pending":false,"status":head.status,"headers":head.headers,"url":head.url})
        }
    }
}
pub(super) fn read(result: ReadResult) -> Value {
    match result {
        ReadResult::Pending => json!({"pending":true}),
        ReadResult::End => json!({"pending":false,"done":true,"chunkBase64":""}),
        ReadResult::Chunk(bytes) => {
            json!({"pending":false,"done":false,"chunkBase64":STANDARD.encode(bytes)})
        }
    }
}
