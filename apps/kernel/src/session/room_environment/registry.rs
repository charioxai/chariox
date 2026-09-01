use std::collections::BTreeMap;

use super::{
    ActionAdmission, CanonicalViewport, EnvironmentActionRequest, EnvironmentActionTerminal,
    EnvironmentActor, EnvironmentComponent, EnvironmentComponentHealthState, EnvironmentError,
    EnvironmentLifecycle, EnvironmentReplay, EnvironmentTabObservation, RoomEnvironment,
    RoomEnvironmentSnapshot,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RoomEnvironmentRegistry {
    environments_by_session: BTreeMap<String, RoomEnvironment>,
}

impl RoomEnvironmentRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn create(
        &mut self,
        session_id: impl Into<String>,
        environment_id: impl Into<String>,
        viewport: CanonicalViewport,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        let session_id = session_id.into();
        if let Some(existing) = self.environments_by_session.get(&session_id) {
            return Err(EnvironmentError::EnvironmentAlreadyExists {
                session_id,
                environment_id: existing.snapshot().environment_id,
            });
        }
        let environment = RoomEnvironment::new(&session_id, environment_id, viewport)?;
        let snapshot = environment.snapshot();
        self.environments_by_session.insert(session_id, environment);
        Ok(snapshot)
    }

    pub(crate) fn start(
        &mut self,
        session_id: &str,
        viewport: CanonicalViewport,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        if !self.environments_by_session.contains_key(session_id) {
            let environment_id = format!("environment-{session_id}");
            self.create(session_id, environment_id, viewport)?;
        }
        let environment = self
            .environments_by_session
            .get_mut(session_id)
            .expect("Room Environment must exist after creation");
        match environment.snapshot().lifecycle {
            EnvironmentLifecycle::Stopped => environment.start_runtime()?,
            EnvironmentLifecycle::Starting | EnvironmentLifecycle::Ready => {}
            from => {
                return Err(EnvironmentError::InvalidLifecycleTransition {
                    from,
                    to: EnvironmentLifecycle::Starting,
                });
            }
        }
        Ok(environment.snapshot())
    }

    pub(crate) fn stop(
        &mut self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.begin_stop(session_id)?;
        self.complete_stop(session_id)
    }

    pub(crate) fn begin_stop(
        &mut self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        let environment = self
            .environments_by_session
            .get_mut(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?;
        match environment.snapshot().lifecycle {
            EnvironmentLifecycle::Stopped
            | EnvironmentLifecycle::Stopping
            | EnvironmentLifecycle::Failed => {}
            _ => environment.transition_to(EnvironmentLifecycle::Stopping)?,
        }
        Ok(environment.snapshot())
    }

    pub(crate) fn complete_stop(
        &mut self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        let environment = self
            .environments_by_session
            .get_mut(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?;
        match environment.snapshot().lifecycle {
            EnvironmentLifecycle::Stopped => {}
            EnvironmentLifecycle::Stopping | EnvironmentLifecycle::Failed => {
                environment.transition_to(EnvironmentLifecycle::Stopped)?;
            }
            from => {
                return Err(EnvironmentError::InvalidLifecycleTransition {
                    from,
                    to: EnvironmentLifecycle::Stopped,
                });
            }
        }
        Ok(environment.snapshot())
    }

    pub(crate) fn retry(
        &mut self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        let environment = self
            .environments_by_session
            .get_mut(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?;
        match environment.snapshot().lifecycle {
            EnvironmentLifecycle::Starting => {}
            EnvironmentLifecycle::Failed | EnvironmentLifecycle::Degraded => {
                environment.start_runtime()?;
            }
            from => {
                return Err(EnvironmentError::InvalidLifecycleTransition {
                    from,
                    to: EnvironmentLifecycle::Starting,
                });
            }
        }
        Ok(environment.snapshot())
    }

    pub(crate) fn transition(
        &mut self,
        session_id: &str,
        lifecycle: EnvironmentLifecycle,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        let environment = self
            .environments_by_session
            .get_mut(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?;
        environment.transition_to(lifecycle)?;
        Ok(environment.snapshot())
    }

    pub(crate) fn update_component_health(
        &mut self,
        session_id: &str,
        component: EnvironmentComponent,
        state: EnvironmentComponentHealthState,
        diagnostic_code: Option<&str>,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        let environment = self
            .environments_by_session
            .get_mut(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?;
        environment.update_component_health(component, state, diagnostic_code);
        Ok(environment.snapshot())
    }

    pub(crate) fn update_viewport_as_actor(
        &mut self,
        session_id: &str,
        actor: EnvironmentActor,
        expected_revision: u64,
        viewport: CanonicalViewport,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        let environment = self
            .environments_by_session
            .get_mut(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?;
        environment.update_viewport_as_actor(actor, expected_revision, viewport)?;
        Ok(environment.snapshot())
    }

    pub(crate) fn reconcile_actors(
        &mut self,
        session_id: &str,
        actors: Vec<EnvironmentActor>,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        let environment = self
            .environments_by_session
            .get_mut(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?;
        environment.reconcile_actors(actors)?;
        Ok(environment.snapshot())
    }

    pub(crate) fn reconcile_controller_tabs(
        &mut self,
        session_id: &str,
        tabs: Vec<EnvironmentTabObservation>,
        focused_runtime_target_id: Option<&str>,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        let environment = self
            .environments_by_session
            .get_mut(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?;
        environment.reconcile_controller_tabs(tabs, focused_runtime_target_id);
        Ok(environment.snapshot())
    }

    pub(crate) fn controller_tab_binding(
        &self,
        session_id: &str,
        tab_id: &str,
    ) -> Result<super::EnvironmentTabRuntimeBinding, EnvironmentError> {
        self.environments_by_session
            .get(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?
            .controller_tab_binding(tab_id)
    }

    pub(crate) fn tab_id_for_controller_target(
        &self,
        session_id: &str,
        controller_target_id: &str,
    ) -> Result<Option<String>, EnvironmentError> {
        Ok(self
            .environments_by_session
            .get(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?
            .tab_id_for_controller_target(controller_target_id))
    }

    pub(crate) fn register_element_references(
        &mut self,
        session_id: &str,
        tab_id: &str,
        runtime_generation: u64,
        document_revision: u64,
        controller_node_refs: impl IntoIterator<Item = String>,
    ) -> Result<BTreeMap<String, String>, EnvironmentError> {
        self.environments_by_session
            .get_mut(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?
            .register_element_references(
                tab_id,
                runtime_generation,
                document_revision,
                controller_node_refs,
            )
    }

    pub(crate) fn resolve_element_reference(
        &self,
        session_id: &str,
        reference_id: &str,
    ) -> Result<super::EnvironmentElementTarget, EnvironmentError> {
        self.environments_by_session
            .get(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?
            .resolve_element_reference(reference_id)
    }

    pub(crate) fn submit_action(
        &mut self,
        session_id: &str,
        request: EnvironmentActionRequest,
    ) -> Result<(ActionAdmission, RoomEnvironmentSnapshot), EnvironmentError> {
        let environment = self
            .environments_by_session
            .get_mut(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?;
        let admission = environment.submit_action(request)?;
        Ok((admission, environment.snapshot()))
    }

    pub(crate) fn existing_action(
        &self,
        session_id: &str,
        request: &EnvironmentActionRequest,
    ) -> Result<Option<ActionAdmission>, EnvironmentError> {
        self.environments_by_session
            .get(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?
            .existing_action(request)
    }

    pub(crate) fn finish_action(
        &mut self,
        session_id: &str,
        action_id: &str,
        terminal: EnvironmentActionTerminal,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        let environment = self
            .environments_by_session
            .get_mut(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?;
        environment.finish_action(action_id, terminal)?;
        Ok(environment.snapshot())
    }

    pub(crate) fn begin_browser_controller_recovery(
        &mut self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        let environment = self
            .environments_by_session
            .get_mut(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?;
        environment.begin_browser_controller_recovery();
        Ok(environment.snapshot())
    }

    pub(crate) fn complete_browser_controller_recovery(
        &mut self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        let environment = self
            .environments_by_session
            .get_mut(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?;
        environment.complete_browser_controller_recovery();
        Ok(environment.snapshot())
    }

    pub(crate) fn request_takeover_as_actor(
        &mut self,
        session_id: &str,
        actor: EnvironmentActor,
        target: super::InputTarget,
    ) -> Result<(super::TakeoverOutcome, RoomEnvironmentSnapshot), EnvironmentError> {
        let environment = self
            .environments_by_session
            .get_mut(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?;
        let outcome = environment.request_takeover_as_actor(actor, target)?;
        Ok((outcome, environment.snapshot()))
    }

    pub(crate) fn release_input(
        &mut self,
        session_id: &str,
        actor_id: &str,
        target: &super::InputTarget,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        let environment = self
            .environments_by_session
            .get_mut(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?;
        environment.release_input(actor_id, target)?;
        Ok(environment.snapshot())
    }

    pub(crate) fn cancel_action_as_actor(
        &mut self,
        session_id: &str,
        actor: EnvironmentActor,
        action_id: &str,
    ) -> Result<(super::ActionCancellationOutcome, RoomEnvironmentSnapshot), EnvironmentError> {
        let environment = self
            .environments_by_session
            .get_mut(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?;
        let outcome = environment.cancel_action_as_actor(actor, action_id)?;
        Ok((outcome, environment.snapshot()))
    }

    pub(crate) fn snapshot(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.environments_by_session
            .get(session_id)
            .map(RoomEnvironment::snapshot)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })
    }

    pub(crate) fn events_after(
        &self,
        session_id: &str,
        cursor: u64,
    ) -> Result<EnvironmentReplay, EnvironmentError> {
        self.environments_by_session
            .get(session_id)
            .map(|environment| environment.events_after(cursor))
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })
    }

    pub(crate) fn action_history(
        &self,
        session_id: &str,
        before_sequence: Option<u64>,
        limit: usize,
    ) -> Result<super::EnvironmentActionHistoryPage, EnvironmentError> {
        self.environments_by_session
            .get(session_id)
            .map(|environment| environment.action_history(before_sequence, limit))
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })
    }

    pub(crate) fn remove(&mut self, session_id: &str) -> Option<RoomEnvironmentSnapshot> {
        self.environments_by_session
            .remove(session_id)
            .map(|environment| environment.snapshot())
    }
}
