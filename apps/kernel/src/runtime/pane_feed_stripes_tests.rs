use super::*;

#[test]
fn delivery_bookkeeping_is_no_blocked_stripe() {
    assert!(is_delivery_bookkeeping("Unconfirmed delivery 3"));
    assert!(is_delivery_bookkeeping("Delivery 2 refusal clock runs"));
    assert!(!is_delivery_bookkeeping("Waiting for the review"));
}

fn waiting(reason: WaitReason, position: u32) -> Waiting {
    Waiting {
        reason,
        position,
        need_bytes: 3 << 30,
        free_bytes: 1 << 30,
        command_line: "cargo build".into(),
        cwd: None,
        since_ms: 0,
    }
}

#[test]
fn a_held_command_says_what_it_waits_for_and_its_place_in_line() {
    assert_eq!(
        command_wait(&waiting(WaitReason::Memory, 1), true),
        "waiting for memory, needs 3 GB"
    );
    assert_eq!(
        command_wait(&waiting(WaitReason::Disk, 2), true),
        "waiting for disk space, 2nd in line"
    );
    assert_eq!(
        command_wait(&waiting(WaitReason::Memory, 12), true),
        "waiting for memory, 12th in line"
    );
    assert_eq!(
        command_wait(&waiting(WaitReason::Memory, 1), false),
        "paused at the memory limit"
    );
}
