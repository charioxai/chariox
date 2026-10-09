//! MP-08/MP-09/MP-10/MP-11: authenticated absence after an idle worker lease is lost.
use super::{LeaseCallerBinding, RemoteLeaseRuntime};
use crate::error::DaemonError;

const CALLER_EVENT: &str = "remote_lease.leased_agent_caller";

impl RemoteLeaseRuntime<'_> {
    pub(super) fn retain_leased_agent_caller(
        &self,
        leased_agent_id: &str,
        caller: &LeaseCallerBinding,
    ) -> Result<(), DaemonError> {
        self.app.durable_state_store().append_event(
            CALLER_EVENT,
            Some(leased_agent_id.to_string()),
            serde_json::json!(caller),
        )?;
        Ok(())
    }

    /// Public caller coordinates only. This proves who may receive an absence
    /// response; it never restores a resource, provider run, or sudo grant.
    pub(super) fn retained_leased_agent_caller(
        &self,
        leased_agent_id: &str,
    ) -> Result<Option<LeaseCallerBinding>, DaemonError> {
        self.app
            .durable_state_store()
            .load_subject_events_by_kind(leased_agent_id, CALLER_EVENT, 1)?
            .pop()
            .map(|event| {
                serde_json::from_value(event.payload).map_err(|error| DaemonError::LocalTransport {
                    operation: "restore leased agent caller",
                    message: error.to_string(),
                })
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{remote_lease::RemoteLeaseRuntime, DaemonApp};

    #[test]
    fn mp10_a10_retained_idle_caller_proves_only_its_absence() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app = DaemonApp::bootstrap(config).unwrap();
        let caller = LeaseCallerBinding {
            home_kernel_id: "home-kernel".into(),
            authenticated_machine_id: "home-machine".into(),
            owner_user_id: crate::session::DEFAULT_LOCAL_USER_ID.into(),
            realm_id: "home-realm".into(),
            public_key_thumbprint: "home-key".into(),
        };
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_bound_execution_lease(
                &caller.home_kernel_id,
                "home-session",
                "home-agent",
                false,
                &caller.owner_user_id,
                caller.clone(),
            )
            .unwrap();
        let agent = RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                Some("sonnet".into()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        assert!(app
            .worker_prompt_receipts
            .caller_for_leased_agent(&agent.id)
            .is_none());
        app.execution_leases.clear();
        app.leased_agents.clear();
        app.execution_lease_callers.clear();
        app.leased_agent_callers.clear();
        app.completed_execution_lease_callers.clear();
        app.completed_leased_agent_callers.clear();
        assert!(RemoteLeaseRuntime::new(&mut app)
            .authorize_leased_agent_caller(&agent.id, &caller)
            .is_ok());
        assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 0);
        for field in 0..5 {
            let mut foreign = caller.clone();
            match field {
                0 => foreign.home_kernel_id = "foreign".into(),
                1 => foreign.authenticated_machine_id = "foreign".into(),
                2 => foreign.owner_user_id = "foreign".into(),
                3 => foreign.realm_id = "foreign".into(),
                _ => foreign.public_key_thumbprint = "foreign".into(),
            }
            assert!(matches!(
                RemoteLeaseRuntime::new(&mut app)
                    .authorize_leased_agent_caller(&agent.id, &foreign),
                Err(DaemonError::LeaseCallerUnauthorized { .. })
            ));
        }
        assert!(matches!(
            RemoteLeaseRuntime::new(&mut app)
                .authorize_leased_agent_caller("unknown-agent", &caller),
            Err(DaemonError::LeasedAgentNotFound { .. })
        ));
        // A currently live resource with no matching owner cannot inherit a
        // durable absence proof from an earlier incarnation of that id.
        app.leased_agents.insert(agent.id.clone(), agent.clone());
        assert!(matches!(
            RemoteLeaseRuntime::new(&mut app).authorize_leased_agent_caller(&agent.id, &caller),
            Err(DaemonError::LeaseCallerUnauthorized { .. })
        ));
        app.leased_agents.clear();
        app.durable_state_store()
            .append_event(
                CALLER_EVENT,
                Some(agent.id.clone()),
                serde_json::json!({"malformed":true}),
            )
            .unwrap();
        assert!(matches!(
            RemoteLeaseRuntime::new(&mut app).authorize_leased_agent_caller(&agent.id, &caller),
            Err(DaemonError::LocalTransport {
                operation: "restore leased agent caller",
                ..
            })
        ));
    }
}
