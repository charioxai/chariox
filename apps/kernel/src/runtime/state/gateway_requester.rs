//! Retained authority for a queued exec submitted by an authenticated gateway.
use super::*;
use crate::runtime::kernel_access::{RemoteRequester, REMOTE_REQUESTER_LEASE};
use crate::runtime::kernel_gateway::origins;

#[derive(Clone)]
pub(super) struct GatewayRequester {
    id: String,
    public_key: String,
    generation: u64,
}

impl KernelRuntimeState {
    pub(crate) fn with_gateway_requester(
        &self,
        origin: &crate::local::OnBehalfOf,
        public_key: &str,
    ) -> Result<Self, DaemonError> {
        let path = origins::path_in(
            &self
                .owned
                .config_projection
                .snapshot()
                .private_runtime_state_root(),
        );
        let generation = origins::admission_generation(&path, public_key)?.ok_or_else(|| {
            crate::runtime::kernel_access::error("the originating gateway is no longer admitted")
        })?;
        let id = format!("{}:{}", origin.origin_kernel, origin.grant_id);
        self.owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .remote_requesters
            .insert(
                id.clone(),
                RemoteRequester {
                    origin_name: origin
                        .origin_name
                        .clone()
                        .unwrap_or_else(|| origin.origin_kernel.clone()),
                    harness: origin.harness.clone(),
                    lease_until: std::time::Instant::now() + REMOTE_REQUESTER_LEASE,
                },
            );
        let mut state = self.clone();
        state.gateway_requester = Some(GatewayRequester {
            id,
            public_key: public_key.to_owned(),
            generation,
        });
        Ok(state)
    }

    pub(super) fn authorize_gateway_requester(&self) -> Result<(), DaemonError> {
        let Some(requester) = &self.gateway_requester else {
            return Ok(());
        };
        let live = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .remote_requesters
            .get(&requester.id)
            .is_some_and(|lease| lease.live(std::time::Instant::now()));
        let path = origins::path_in(
            &self
                .owned
                .config_projection
                .snapshot()
                .private_runtime_state_root(),
        );
        if !live
            || origins::admission_generation(&path, &requester.public_key)?
                != Some(requester.generation)
        {
            return Err(crate::runtime::kernel_access::error(
                "the original gateway grant ended before the command started",
            ));
        }
        Ok(())
    }

    /// Whether `acting_agent` names an agent of the kernel this request came from
    /// (`<origin kernel>/<agent>`). Only a request of an admitted gateway can carry
    /// one, and only under the origin name this kernel verified for it, so a sender
    /// cannot make a process look like another kernel's agent. It labels a process;
    /// the gateway grant is what authorizes the request.
    pub(super) fn names_origin_agent(&self, acting_agent: &str) -> bool {
        let Some(requester) = &self.gateway_requester else {
            return false;
        };
        let access = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned");
        access
            .remote_requesters
            .get(&requester.id)
            .is_some_and(|known| {
                acting_agent
                    .strip_prefix(known.origin_name.as_str())
                    .and_then(|rest| rest.strip_prefix('/'))
                    .is_some_and(|agent| !agent.is_empty())
            })
    }
}
