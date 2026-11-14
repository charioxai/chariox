//! Pinned allocation targets never enter the owner's machine directory.
use super::{validate_machine_target, GatewayError};
use chariox_relay::protocol::RelayKernelPresence;
use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Default)]
pub(super) struct JobTargets(Mutex<BTreeMap<String, RelayKernelPresence>>);

impl JobTargets {
    pub(super) fn bind(
        &self,
        target: &crate::local::ControlPlaneMachineTarget,
    ) -> Result<(), GatewayError> {
        let presence = target.presence.as_ref().ok_or_else(|| {
            refused(
                &target.kernel_id,
                "job_target_pending",
                "The remote job machine has not advertised its runtime capabilities yet",
            )
        })?;
        validate_machine_target(target, presence)?;
        if !["kernel_gateway_v1", "room_terminal_v1", "room_exec_v1"]
            .into_iter()
            .all(|capability| {
                presence
                    .capabilities
                    .iter()
                    .any(|found| found == capability)
            })
        {
            return Err(refused(
                &target.kernel_id,
                "job_target_pending",
                "The remote job machine has not advertised the required execution capabilities",
            ));
        }
        let mut targets = self.0.lock().expect("job targets");
        if targets.get(&target.kernel_id).is_some_and(|previous| {
            previous.public_key != presence.public_key || previous.machine_id != presence.machine_id
        }) {
            return Err(refused(&target.kernel_id, "control_plane_binding_invalid", "The pinned job machine identity changed; cancel this job and allocate a new machine"));
        }
        targets.insert(target.kernel_id.clone(), presence.clone());
        Ok(())
    }
    pub(super) fn get(&self, kernel_id: &str) -> Option<RelayKernelPresence> {
        self.0.lock().expect("job targets").get(kernel_id).cloned()
    }
    pub(super) fn remove(&self, kernel_id: &str) {
        self.0.lock().expect("job targets").remove(kernel_id);
    }
}
fn refused(kernel_id: &str, code: &str, message: &str) -> GatewayError {
    GatewayError::Remote {
        name: kernel_id.into(),
        code: code.into(),
        message: message.into(),
        retryable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target() -> crate::local::ControlPlaneMachineTarget {
        let presence = RelayKernelPresence {
            kernel_id: "private-worker".into(),
            machine_id: "worker-machine".into(),
            machine_alias: None,
            relay_alias: None,
            kernel_alias: None,
            available_providers: vec![],
            provider_accounts: vec![],
            capabilities: vec![
                "kernel_gateway_v1".into(),
                "room_terminal_v1".into(),
                "room_exec_v1".into(),
            ],
            accepting_remote_leases: false,
            leased_agent_count: 0,
            local_session_count: 0,
            public_key: "enrolled-worker-key".into(),
        };
        crate::local::ControlPlaneMachineTarget {
            kernel_id: presence.kernel_id.clone(),
            machine_id: presence.machine_id.clone(),
            relay_realm_id: "realm".into(),
            key_thumbprint: crate::runtime::terminal_pairings::public_key_thumbprint(
                &presence.public_key,
            ),
            presence: Some(presence),
        }
    }
    #[test]
    fn offload_job_private_target_requires_canonical_pin_and_never_resolves_aliases() {
        let targets = JobTargets::default();
        let mut target = target();
        target.presence.as_mut().unwrap().machine_alias = Some("user-machine-name".into());
        targets.bind(&target).unwrap();
        assert!(targets.get("private-worker").is_some());
        assert!(targets.get("user-machine-name").is_none());
        let mut changed = target.clone();
        changed.presence.as_mut().unwrap().public_key = "replacement-key".into();
        assert_eq!(
            targets.bind(&changed).unwrap_err().code(),
            "control_plane_binding_invalid"
        );
        changed.key_thumbprint =
            crate::runtime::terminal_pairings::public_key_thumbprint("replacement-key");
        assert_eq!(
            targets.bind(&changed).unwrap_err().code(),
            "control_plane_binding_invalid"
        );
        targets.remove("private-worker");
        assert!(targets.get("private-worker").is_none());
    }
    #[test]
    fn offload_job_private_target_waits_for_capabilities_without_inventory_fallback() {
        let targets = JobTargets::default();
        let mut target = target();
        target.presence = None;
        assert_eq!(
            targets.bind(&target).unwrap_err().code(),
            "job_target_pending"
        );
        assert!(targets.get("private-worker").is_none());
    }
}
