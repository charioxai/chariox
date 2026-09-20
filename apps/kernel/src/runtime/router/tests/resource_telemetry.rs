use crate::local::test_support::LocalRouterTestHarness;
use crate::local::{GetKernelResourceTelemetryRequest, LocalDaemonRequest, LocalDaemonResponse};

#[test]
fn router_serves_kernel_authoritative_resource_telemetry() {
    std::fs::create_dir_all(crate::logging::default_log_root())
        .expect("kernel log root should be available");
    let harness = LocalRouterTestHarness::new();
    let response = harness
        .dispatch(LocalDaemonRequest::GetKernelResourceTelemetry(
            GetKernelResourceTelemetryRequest,
        ))
        .expect("resource telemetry should be served by the kernel router");
    let LocalDaemonResponse::KernelResourceTelemetry { snapshot } = response else {
        panic!("unexpected resource telemetry response");
    };

    assert!(snapshot.telemetry.authoritative);
    assert_eq!(snapshot.telemetry.scope, "managed-target");
    assert_eq!(snapshot.telemetry.source, "kernel");
    assert!(snapshot.memory.used_bytes <= snapshot.memory.total_bytes);
    assert!(snapshot.memory.available_bytes <= snapshot.memory.total_bytes);
    assert!(snapshot.disk.used_bytes <= snapshot.disk.total_bytes);
    assert!(snapshot.disk.available_bytes <= snapshot.disk.total_bytes);
    assert!(snapshot.process.count > 0);
}
