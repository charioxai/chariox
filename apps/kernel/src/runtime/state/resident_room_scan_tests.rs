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

/// What building the Room list allocates, and how many Rooms stay resident, once 20 of 24 ended.
async fn room_list_after_20_of_24_ended() -> (usize, usize) {
    let runtime = runtime_with_rooms(24);
    let rooms = runtime.owned.session_store.list_all_sessions();
    for room in rooms.iter().take(20) {
        runtime
            .end_session(room.id())
            .await
            .expect("the Room should end");
    }
    let resident = runtime.owned.session_store.list_all_sessions().len();
    let (_, listed) = measured(async { runtime.list_session_snapshots() }).await;
    (resident, listed.allocated)
}

#[tokio::test]
async fn ended_rooms_leave_the_resident_store_and_cost_the_room_list_nothing() {
    let (resident, ended_allocated) = room_list_after_20_of_24_ended().await;
    assert_eq!(
        resident, 4,
        "an ended Room stayed resident in the session store"
    );
    let (_, four_allocated) = {
        let four = runtime_with_rooms(4);
        measured(async { four.list_session_snapshots() }).await
    };
    assert!(
        ended_allocated <= four_allocated.allocated + SCAN_OVERHEAD,
        "the list allocated {} bytes with 4 Rooms and {ended_allocated} with 4 live and 20 ended",
        four_allocated.allocated
    );
}
