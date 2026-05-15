use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{LaunchProviderRequest, ProviderRunState, RuntimeProviderRun};

use super::provider_liveness::clear_active_provider_run_session_pointer;

#[derive(Debug, Clone)]
pub(crate) struct StartedProviderLaunch {
    pub(crate) run: RuntimeProviderRun,
    pub(crate) previous_active_run_id: Option<String>,
}

pub(super) struct ProviderRunActivationState;

impl ProviderRunActivationState {
    pub(super) fn start_provider_run_for_session(
        app: &mut DaemonApp,
        request: LaunchProviderRequest,
    ) -> Result<StartedProviderLaunch, DaemonError> {
        let session_id = request.session_id.clone();
        let previous_active_run_id = app
            .sessions
            .get_session(&session_id)?
            .active_provider_run_id()
            .map(str::to_owned);

        if let Some(active_run_id) = previous_active_run_id.as_deref() {
            let active_run = app.providers.get_run(active_run_id)?;
            match active_run.state() {
                ProviderRunState::Ended => {
                    app.sessions.set_active_provider_run(&session_id, None)?;
                    app.providers.clear_runtime(active_run_id);
                }
                ProviderRunState::Starting => {
                    let outcome = app
                        .providers
                        .terminate_run_provider_only(&session_id, active_run_id)?;
                    clear_active_provider_run_session_pointer(
                        app,
                        &session_id,
                        outcome.run().id(),
                    )?;
                    app.update_provider_run_projection(outcome.into_run());
                }
                ProviderRunState::Running => {
                    if !app.provider_run_has_active_prompt(&session_id, &active_run)? {
                        let outcome = app
                            .providers
                            .park_run_provider_only(&session_id, active_run_id)?;
                        clear_active_provider_run_session_pointer(
                            app,
                            &session_id,
                            outcome.run().id(),
                        )?;
                        app.update_provider_run_projection(outcome.into_run());
                    }
                }
                ProviderRunState::Parked => {
                    app.sessions.set_active_provider_run(&session_id, None)?;
                }
            }
        }

        let outcome = app.providers.start_run_provider_only(request)?;
        app.sessions
            .set_active_provider_run(&session_id, Some(outcome.run().id().to_string()))?;
        Ok(StartedProviderLaunch {
            run: outcome.into_run(),
            previous_active_run_id,
        })
    }

    pub(super) fn resume_provider_run_for_session(
        app: &mut DaemonApp,
        session_id: &str,
        run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let active_run_id = app
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_owned);

        if let Some(active_run_id) = active_run_id.as_deref() {
            if active_run_id != run_id {
                let active_run = app.providers.get_run(active_run_id)?;
                match active_run.state() {
                    ProviderRunState::Running => {
                        if !app.provider_run_has_active_prompt(session_id, &active_run)? {
                            let outcome = app
                                .providers
                                .park_run_provider_only(session_id, active_run_id)?;
                            clear_active_provider_run_session_pointer(
                                app,
                                session_id,
                                outcome.run().id(),
                            )?;
                            app.update_provider_run_projection(outcome.into_run());
                        }
                    }
                    ProviderRunState::Starting => {
                        let outcome = app
                            .providers
                            .terminate_run_provider_only(session_id, active_run_id)?;
                        clear_active_provider_run_session_pointer(
                            app,
                            session_id,
                            outcome.run().id(),
                        )?;
                        app.update_provider_run_projection(outcome.into_run());
                    }
                    ProviderRunState::Parked | ProviderRunState::Ended => {
                        app.sessions.set_active_provider_run(session_id, None)?;
                    }
                }
            }
        }

        let outcome = app.providers.resume_run_provider_only(session_id, run_id)?;
        app.sessions
            .set_active_provider_run(session_id, Some(outcome.run().id().to_string()))?;
        let run = outcome.into_run();
        app.update_provider_run_projection(run.clone());
        Ok(run)
    }
}
