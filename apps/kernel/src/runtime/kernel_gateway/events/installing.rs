//! MP-08 / MP-09 / MP-10 / MP-11: one target the kernel refuses must not stop the others.
use super::*;

impl KernelGateway {
    /// Installs the followed `targets` on `kernel`. When the kernel refuses them for good, the
    /// targets are installed one by one: a refused one ended before it was installed. Such a
    /// target is dropped and the follower is told, so the others keep being pulled.
    pub(super) async fn install_event_targets(
        &self,
        origin: &Origin,
        kernel: &str,
        targets: &[EventTarget],
    ) -> Result<(), GatewayError> {
        let refusal = match self.install(origin, kernel, targets).await {
            Err(
                refusal @ GatewayError::Remote {
                    retryable: false, ..
                },
            ) => refusal,
            other => return other,
        };
        let mut refused = Vec::new();
        for target in targets {
            match self
                .install(origin, kernel, std::slice::from_ref(target))
                .await
            {
                Err(GatewayError::Remote {
                    retryable: false, ..
                }) => refused.push(target),
                other => other?,
            }
        }
        if refused.is_empty() {
            return Err(refusal);
        }
        for target in refused {
            self.runtime_state
                .outside_store()
                .outside_mutate(Mutation::Publish {
                    occurrence: format!(
                        "target-refused:{}",
                        serde_json::to_string(target).expect("event target serializes")
                    ),
                    event: Box::new(refused_target_ended(target)),
                })
                .map_err(|_| part_error(kernel, "could not drop a target the kernel refused"))?;
        }
        Ok(())
    }

    async fn install(
        &self,
        origin: &Origin,
        kernel: &str,
        targets: &[EventTarget],
    ) -> Result<(), GatewayError> {
        self.forward(
            kernel,
            origin,
            &event_command_id(),
            &LocalDaemonRequest::SubscribeEvents(SubscribeEventsRequest {
                targets: targets.to_vec(),
            }),
            Duration::from_secs(1),
        )
        .await
        .map(drop)
    }
}

/// What the follower is told about a target its kernel no longer accepts: the session, or just
/// the agent, is gone.
fn refused_target_ended(target: &EventTarget) -> OutsideEvent {
    OutsideEvent {
        cursor: 0,
        kernel: target.kernel.clone(),
        machine: String::new(),
        session_id: target.session_id.clone(),
        agent_id: target.agent_id.clone(),
        value: OutsideEventValue::Session {
            state: if target.agent_id.is_some() {
                EventSessionState::AgentRemoved
            } else {
                EventSessionState::Ended
            },
        },
        summary: "the kernel no longer accepts this target".into(),
        at_ms: crate::session::unix_epoch_ms(),
        refs: Default::default(),
    }
}
