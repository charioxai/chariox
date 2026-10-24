//! Acknowledged delivery of a leased agent's runtime projection to its home.
//!
//! A drain consumes the worker's output records, so a reply lost on the way to the
//! home (a relay writer closing at reconnect, a home timeout) would lose that output.
//! The drained projection therefore stays here, numbered, until the home's next drain
//! acknowledges its number; until then every drain answers with the same projection.
use crate::error::DaemonError;
use crate::transport::relay_peer::RelayPeerEvent;

use super::RemoteLeaseRuntime;

#[derive(Debug, Default)]
pub(crate) struct LeasedProjectionDelivery {
    last_sequence: u64,
    unacknowledged: Option<RetainedProjection>,
}

#[derive(Debug)]
struct RetainedProjection {
    sequence: u64,
    provider_run_id: String,
    event: RelayPeerEvent,
}

#[derive(Debug)]
pub(crate) struct DrainedLeasedProjection {
    pub(crate) event: RelayPeerEvent,
    /// The home acknowledges this number before the next projection is drained.
    pub(crate) sequence: u64,
}

impl RemoteLeaseRuntime<'_> {
    /// `acknowledged` is the last sequence the home applied for this leased agent.
    pub(crate) fn drain_acknowledged_leased_runtime_projection(
        &mut self,
        leased_agent_id: &str,
        provider_run_id: &str,
        pump_output: bool,
        replay_settled_completion: bool,
        acknowledged: u64,
    ) -> Result<Option<DrainedLeasedProjection>, DaemonError> {
        let delivery = self
            .app
            .leased_projection_deliveries
            .entry(leased_agent_id.to_string())
            .or_default();
        // A restarted worker numbers from zero again; continuing after the home's
        // acknowledgement keeps a fresh projection from counting as already applied.
        delivery.last_sequence = delivery.last_sequence.max(acknowledged);
        if delivery
            .unacknowledged
            .as_ref()
            .is_some_and(|retained| retained.sequence <= acknowledged)
        {
            delivery.unacknowledged = None;
        }
        // A projection of another run is from a turn the home has already settled
        // without it; the home only applies projections for the run it asks about.
        if let Some(retained) = delivery
            .unacknowledged
            .as_ref()
            .filter(|retained| retained.provider_run_id == provider_run_id)
        {
            return Ok(Some(DrainedLeasedProjection {
                event: retained.event.clone(),
                sequence: retained.sequence,
            }));
        }
        let Some((_target_daemon_id, event)) = self.drain_leased_runtime_projection_with_recovery(
            leased_agent_id,
            provider_run_id,
            pump_output,
            replay_settled_completion,
        )?
        else {
            return Ok(None);
        };
        let delivery = self
            .app
            .leased_projection_deliveries
            .entry(leased_agent_id.to_string())
            .or_default();
        delivery.last_sequence += 1;
        let sequence = delivery.last_sequence;
        delivery.unacknowledged = Some(RetainedProjection {
            sequence,
            provider_run_id: provider_run_id.to_string(),
            event: event.clone(),
        });
        Ok(Some(DrainedLeasedProjection { event, sequence }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DaemonConfig;
    use crate::terminal::TerminalOutputKind;

    fn leased_app_with_output(
        text: &[u8],
    ) -> (
        crate::app::DaemonApp,
        crate::execution_lease::LeasedAgent,
        String,
    ) {
        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app = crate::app::DaemonApp::bootstrap(config).expect("daemon should bootstrap");
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease("home-kernel", "session-1", "agent-home-1", false, "user")
            .expect("execution lease should be created");
        let leased = RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("leased agent should be created");
        let (run, _) = RemoteLeaseRuntime::new(&mut app)
            .submit_leased_prompt(&leased.id, "remote leased prompt\n", Vec::new())
            .expect("leased prompt should submit");
        emit_output(&mut app, &leased, &run, text);
        (app, leased, run)
    }

    fn emit_output(
        app: &mut crate::app::DaemonApp,
        leased: &crate::execution_lease::LeasedAgent,
        run: &str,
        text: &[u8],
    ) {
        app.terminal_mut().fan_out_output(
            &leased.backing_session_id,
            run,
            Some(&leased.backing_agent_id),
            TerminalOutputKind::ProviderOutput,
            Some(format!("output-{}", String::from_utf8_lossy(text))),
            vec![leased.backing_attachment_id.clone()],
            text,
        );
    }

    fn drain(
        app: &mut crate::app::DaemonApp,
        leased: &crate::execution_lease::LeasedAgent,
        run: &str,
        acknowledged: u64,
    ) -> Option<(Vec<String>, u64)> {
        RemoteLeaseRuntime::new(app)
            .drain_acknowledged_leased_runtime_projection(
                &leased.id,
                run,
                false,
                true,
                acknowledged,
            )
            .expect("drain should succeed")
            .map(|drained| {
                let RelayPeerEvent::LeasedRuntimeProjection { output_chunks, .. } = drained.event;
                let texts = output_chunks
                    .into_iter()
                    .filter(|chunk| chunk.kind == TerminalOutputKind::ProviderOutput)
                    .map(|chunk| String::from_utf8_lossy(&chunk.bytes).into_owned())
                    .collect();
                (texts, drained.sequence)
            })
    }

    // MP-08 / MP-10 A10-S02: a 170 s relay partition lost the final PART_DONE text
    // because the drain reply that carried it died with the worker's relay writer.
    #[test]
    fn a_drain_reply_lost_on_the_way_home_is_answered_again() {
        let (mut app, leased, run) = leased_app_with_output(b"PART_DONE");
        let first = drain(&mut app, &leased, &run, 0).expect("the output is drained");
        assert_eq!(first, (vec!["PART_DONE".to_string()], 1));

        // The reply was lost, so the home still acknowledges nothing.
        let again = drain(&mut app, &leased, &run, 0)
            .expect("an unacknowledged projection must be answered again");
        assert_eq!(again, first, "the same projection, under the same number");
    }

    #[test]
    fn an_acknowledged_projection_is_not_sent_twice() {
        let (mut app, leased, run) = leased_app_with_output(b"FIRST");
        assert_eq!(
            drain(&mut app, &leased, &run, 0),
            Some((vec!["FIRST".to_string()], 1))
        );
        // Output that arrives while a projection is unacknowledged waits for the next one.
        emit_output(&mut app, &leased, &run, b"SECOND");
        assert_eq!(
            drain(&mut app, &leased, &run, 0),
            Some((vec!["FIRST".to_string()], 1))
        );
        assert_eq!(
            drain(&mut app, &leased, &run, 1),
            Some((vec!["SECOND".to_string()], 2))
        );
        let after = drain(&mut app, &leased, &run, 2).map(|(texts, _)| texts);
        assert!(
            after.unwrap_or_default().is_empty(),
            "nothing is sent twice"
        );
    }

    #[test]
    fn a_restarted_worker_numbers_after_the_homes_acknowledgement() {
        // The home applied five projections before the worker restarted.
        let (mut app, leased, run) = leased_app_with_output(b"AFTER_RESTART");
        assert_eq!(
            drain(&mut app, &leased, &run, 5),
            Some((vec!["AFTER_RESTART".to_string()], 6))
        );
        assert_eq!(
            drain(&mut app, &leased, &run, 5),
            Some((vec!["AFTER_RESTART".to_string()], 6)),
            "a lost reply after the restart is answered again"
        );
    }
}
