use super::ProviderRunOperationLanes;

// MP-08 / MP-09 / MP-10 / MP-11: a lane wait over two seconds names the waiter and the holder.
#[tokio::test(start_paused = true)]
async fn a_long_lane_wait_names_the_waiter_and_the_holder() {
    let lanes = ProviderRunOperationLanes::with_wait_trace_for_tests();
    let held = lanes.acquire("run-1").await;
    let waiter = tokio::spawn({
        let lanes = lanes.clone();
        async move {
            drop(lanes.acquire("run-1").await);
        }
    });
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let reports = lanes.wait_trace.reports();
    assert_eq!(reports.len(), 1, "{reports:?}");
    let (waiter_sites, holder_sites) = reports[0]
        .split_once(" holder: ")
        .expect("report names the holder");
    let test_frame = "wait_trace_tests::a_long_lane_wait_names_the_waiter_and_the_holder";
    assert!(waiter_sites.starts_with("run-1 waiter: "), "{reports:?}");
    assert!(waiter_sites.contains(test_frame), "{reports:?}");
    assert!(holder_sites.contains(test_frame), "{reports:?}");
    drop(held);
    waiter.await.unwrap();
    assert_eq!(lanes.wait_trace.reports().len(), 1);
}
