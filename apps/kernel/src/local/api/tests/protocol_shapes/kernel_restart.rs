//! Row 551: the owner's request to restart a kernel (a hot reload of it).
use super::*;

#[test]
fn restart_kernel_is_a_kernel_request_and_kernel_restarting_its_answer() {
    let request = serde_json::json!({"RestartKernel": {}});
    let value: LocalDaemonRequest =
        serde_json::from_value(request.clone()).expect("RestartKernel is a kernel request");
    assert_eq!(serde_json::to_value(value).unwrap(), request);
    let answer = serde_json::json!({"KernelRestarting": {"hot_reload": true}});
    let value: LocalDaemonResponse =
        serde_json::from_value(answer.clone()).expect("KernelRestarting is a kernel response");
    assert_eq!(serde_json::to_value(value).unwrap(), answer);
}
