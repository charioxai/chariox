//! MP-08 / MP-09 / MP-10 / MP-11: the reconcile scans that run on a timer must not
//! copy the Rooms they pass over. A kernel that keeps more Rooms resident does
//! the same allocation work as one that keeps few, when none of them has a match.
use super::*;
use crate::test_support::allocation::{measured, runtime_with_rooms};

/// What a scan may allocate on its own, whatever the number of Rooms.
const SCAN_OVERHEAD: usize = 4096;

#[tokio::test]
async fn http_trigger_reconcile_allocates_nothing_per_room() {
    let (few, many) = (runtime_with_rooms(8), runtime_with_rooms(48));
    let (_, few) = measured(few.reconcile_agent_trigger_gateways()).await;
    let (_, many) = measured(many.reconcile_agent_trigger_gateways()).await;
    assert!(
        many.allocated <= few.allocated + SCAN_OVERHEAD,
        "the scan allocated {} bytes with 8 Rooms and {} with 48",
        few.allocated,
        many.allocated
    );
}

#[tokio::test]
async fn inactive_steer_reconcile_allocates_nothing_per_room() {
    let (few, many) = (runtime_with_rooms(8), runtime_with_rooms(48));
    let (_, few) = measured(async { few.owned.reconcile_inactive_local_steers() }).await;
    let (_, many) = measured(async { many.owned.reconcile_inactive_local_steers() }).await;
    assert!(
        many.allocated <= few.allocated + SCAN_OVERHEAD,
        "the scan allocated {} bytes with 8 Rooms and {} with 48",
        few.allocated,
        many.allocated
    );
}
