use std::collections::BTreeMap;

use super::{
    CanonicalViewport, EnvironmentActor, EnvironmentError, EnvironmentLifecycle, RoomEnvironment,
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
        let environment = self
            .environments_by_session
            .get_mut(session_id)
            .ok_or_else(|| EnvironmentError::EnvironmentNotFound {
                session_id: session_id.to_string(),
            })?;
        match environment.snapshot().lifecycle {
            EnvironmentLifecycle::Stopped => {}
            EnvironmentLifecycle::Stopping => {
                environment.transition_to(EnvironmentLifecycle::Stopped)?;
            }
            EnvironmentLifecycle::Failed => {
                environment.transition_to(EnvironmentLifecycle::Stopped)?;
            }
            _ => {
                environment.transition_to(EnvironmentLifecycle::Stopping)?;
                environment.transition_to(EnvironmentLifecycle::Stopped)?;
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

    // The managed controller adapter reports lifecycle completion in Milestone 2.
    #[cfg_attr(not(test), allow(dead_code))]
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

    pub(crate) fn remove(&mut self, session_id: &str) -> Option<RoomEnvironmentSnapshot> {
        self.environments_by_session
            .remove(session_id)
            .map(|environment| environment.snapshot())
    }
}
