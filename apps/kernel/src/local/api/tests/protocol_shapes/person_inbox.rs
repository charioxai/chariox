//! Local 1012: the person's Inbox (docs/NOTIFICATION_CENTER_PLAN.md section 4): its cards, read, snooze and mute.
use crate::local::*;
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn person_inbox_1012_snapshot_and_hash() {
    let bytes = include_bytes!("person-inbox-1012.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "d86e4140e5e85417c2b8c4b8a3d39ffefd3d6623a6f295cd81f2a919ee5db131"
