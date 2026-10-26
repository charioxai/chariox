//! Worker destruction shares the connected relay route, but never needs to
//! hold the home app lock while the worker acknowledges its cleanup.
use super::*;
use crate::transport::relay_client::ConnectedRelays;

pub(crate) struct RemoteAgentCleanupPlan {
    binding: RemoteAgentBinding,
    config: DaemonConfig,
    relays: ConnectedRelays,
}

pub(crate) struct RemoteAgentCleanupResult {
    binding: RemoteAgentBinding,
    receipts: Vec<Result<CleanupReceipt, DaemonError>>,
}

enum CleanupReceipt {
    Destroyed,
    AlreadyAbsent {
        resource_id: String,
        code: &'static str,
    },
}

impl RemoteAgentCleanupPlan {
    pub(crate) async fn execute(
        self,
        authorize: &(dyn Fn() -> Result<(), DaemonError> + Send + Sync),
    ) -> RemoteAgentCleanupResult {
        let agent = self
            .request(
                RelayPeerRequest::DestroyLeasedAgent {
                    leased_agent_id: self.binding.leased_agent_id.clone(),
                },
                &self.binding.leased_agent_id,
                "leased_agent_not_found",
                authorize,
            )
            .await;
        let agent_destroyed = agent.is_ok();
        let mut receipts = vec![agent];
        if agent_destroyed {
            // The agent is already gone; its grant cannot gate lease cleanup.
            receipts.push(
                self.request(
                    RelayPeerRequest::DestroyExecutionLease {
                        lease_id: self.binding.execution_lease_id.clone(),
                    },
                    &self.binding.execution_lease_id,
                    "execution_lease_not_found",
                    &|| Ok(()),
                )
                .await,
            );
        }
        RemoteAgentCleanupResult {
            binding: self.binding,
            receipts,
        }
    }

    async fn request(
        &self,
        request: RelayPeerRequest,
        resource_id: &str,
        missing_code: &'static str,
        authorize: &(dyn Fn() -> Result<(), DaemonError> + Send + Sync),
    ) -> Result<CleanupReceipt, DaemonError> {
        let response = async {
            self.relays
                .enqueue_leased_worker_request_authorized(
                    &self.config,
                    ClientTarget {
                        daemon_id: Some(self.binding.worker_kernel_id.clone()),
                        daemon_alias: None,
                    },
                    request,
                    Duration::from_millis(self.config.relay_request_timeout_ms),
                    authorize,
                )
                .await?
                .wait()
                .await
        }
        .await;
        validate_response(&self.binding, resource_id, missing_code, response)
    }
}

impl DaemonApp {
    pub(crate) fn prepare_remote_execution_cleanup(
        &self,
        binding: &RemoteAgentBinding,
    ) -> RemoteAgentCleanupPlan {
        RemoteAgentCleanupPlan {
            binding: binding.clone(),
            config: self.relay_config_for_remote_execution(binding),
            relays: self.connected_relays(),
        }
    }

    pub(crate) fn commit_remote_execution_cleanup(
        &self,
        result: RemoteAgentCleanupResult,
    ) -> Result<(), DaemonError> {
        for receipt in result.receipts {
            self.commit_cleanup_receipt(&result.binding, receipt?)?;
        }
        Ok(())
    }

    fn commit_cleanup_receipt(
        &self,
        binding: &RemoteAgentBinding,
        receipt: CleanupReceipt,
    ) -> Result<(), DaemonError> {
        if let CleanupReceipt::AlreadyAbsent { resource_id, code } = receipt {
            self.durable_state_store().append_event(
                "agent.worker_cleanup.already_absent",
                None,
                serde_json::json!({
                    "worker_kernel_id": binding.worker_kernel_id,
                    "execution_lease_id": binding.execution_lease_id,
                    "leased_agent_id": binding.leased_agent_id,
                    "missing_resource_id": resource_id,
                    "code": code,
                }),
            )?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn reconcile_remote_cleanup_response(
        &self,
        binding: &RemoteAgentBinding,
        resource_id: &str,
        missing_code: &'static str,
        response: Result<RelayPeerResponse, DaemonError>,
    ) -> Result<(), DaemonError> {
        self.commit_cleanup_receipt(
            binding,
            validate_response(binding, resource_id, missing_code, response)?,
        )
    }
}

fn validate_response(
    binding: &RemoteAgentBinding,
    resource_id: &str,
    missing_code: &'static str,
    response: Result<RelayPeerResponse, DaemonError>,
) -> Result<CleanupReceipt, DaemonError> {
    match response {
        Ok(RelayPeerResponse::LeasedAgentDestroyed { leased_agent_id })
            if missing_code == "leased_agent_not_found" && leased_agent_id == resource_id =>
        {
            Ok(CleanupReceipt::Destroyed)
        }
        Ok(RelayPeerResponse::ExecutionLeaseDestroyed { lease_id })
            if missing_code == "execution_lease_not_found" && lease_id == resource_id =>
        {
            Ok(CleanupReceipt::Destroyed)
        }
        Err(DaemonError::RelayPeerCleanupAbsent {
            worker_kernel_id,
            resource_id: absent_id,
            code,
        }) if worker_kernel_id == binding.worker_kernel_id
            && absent_id == resource_id
            && code == missing_code =>
        {
            Ok(CleanupReceipt::AlreadyAbsent {
                resource_id: absent_id,
                code,
            })
        }
        Err(error) => Err(error),
        Ok(other) => Err(DaemonError::LocalTransport {
            operation: "destroy remote execution binding",
            message: format!("unexpected peer cleanup response: {other:?}"),
        }),
    }
}
