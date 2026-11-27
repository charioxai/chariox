use super::KernelRuntimeState;
use crate::local::{GetDaemonHealthRequest, LocalDaemonRequest};
use serde_json::{json, Value};
use std::time::Duration;

impl KernelRuntimeState {
    pub(super) async fn target_build_toolchain(
        &self,
        kernel_id: &str,
        origin: &crate::runtime::kernel_gateway::Origin,
    ) -> Value {
        let unavailable = || json!({"state": "unavailable"});
        let Ok(grant) = self.machine_grant_for_origin(kernel_id, origin) else {
            return unavailable();
        };
        let request = LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest);
        let id = format!("machines:build-toolchain:{:032x}", rand::random::<u128>());
        let Ok(Ok(response)) =
            tokio::time::timeout(Duration::from_secs(5), grant.send(id, &request)).await
        else {
            return unavailable();
        };
        match response.pointer("/DaemonHealth/projection/build_toolchain_version") {
            Some(Value::String(version)) => json!({"state": "installed", "version": version}),
            Some(Value::Null) => json!({"state": "not_installed"}),
            _ => unavailable(),
        }
    }
}
