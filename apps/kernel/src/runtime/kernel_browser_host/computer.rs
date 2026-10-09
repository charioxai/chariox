//! MP-08 / MP-11: Desktop scope and input admission over the existing host service.
use super::*;
impl KernelBrowserHost {
    pub(super) fn computer_request(
        &self,
        user: &str,
        admission: Option<&KernelBrowserAdmission>,
        mut params: Value,
        policy: Value,
    ) -> Result<Value, crate::error::HostFailure> {
        self.check_admission(admission)?;
        if params["op"] == "display_subscribe" && admission.is_some_and(|a| a.agent.is_some()) {
            return Err("MP-11: not_granted: desktop video requires a human terminal".into());
        }
        if admission.is_some_and(|a| {
            a.user != user
                || (a.agent.is_some() && a.capability != KernelBrowserCapability::Computer)
        }) {
            return Err("MP-11: not_granted: Computer capability required".into());
        }
        if matches!(
            params["op"].as_str(),
            Some("takeover" | "release" | "actors")
        ) {
            if admission.is_some_and(|a| a.agent.is_some()) {
                return Err("MP-11: not_granted: human desktop control required".into());
            }
            let model = self.actor_model(user)?;
            let mut model = model
                .lock()
                .map_err(|_| "MP-11: desktop actor lock unavailable")?;
            if admission.is_some_and(|a| a.revoked_in_actor_lock()) {
                return Err("MP-11: not_granted: terminal desktop authority retired".into());
            }
            if params["op"] == "actors" {
                model.retire_lapsed_desktop_viewers(std::time::Instant::now());
                return Ok(model.snapshot());
            }
            let actor = browser_actor(admission, &params);
            let surface = params["surface_id"]
                .as_str()
                .ok_or("MP-11: stale native surface")?;
            let generation = params["generation"]
                .as_str()
                .ok_or("MP-11: stale native generation")?;
            if params["op"] == "takeover" {
                let result = model.takeover_desktop(actor.clone(), surface, generation)?;
                drop(model);
                // The cancellation token interrupts in-flight native holds while this
                // backend lock waits. Reset old persistent keys before acknowledging.
                let backend = self.backend(user)?;
                let mut backend = backend
                    .lock()
                    .map_err(|_| "MP-11: desktop backend lock unavailable")?;
                self.check_admission(admission)?;
                if !self
                    .actor_model(user)?
                    .lock()
                    .map_err(|_| "MP-11: desktop actor lock unavailable")?
                    .desktop_owner_is(&actor.actor_id)
                {
                    return Err("MP-11: not_granted: desktop takeover superseded".into());
                }
                backend.host_request_classified("host.computer.reset", serde_json::json!({}))?;
                self.check_admission(admission)?;
                return serde_json::to_value(result)
                    .map_err(|_| "MP-11: invalid takeover outcome".into());
            }
            // Keep ownership until native keys are reset. Another actor cannot
            // begin input while the release waits for in-flight controller work.
            model.check_desktop_release(&actor.actor_id, surface, generation)?;
            drop(model);
            let backend = self.backend(user)?;
            let mut backend = backend
                .lock()
                .map_err(|_| "MP-11: desktop backend lock unavailable")?;
            self.check_admission(admission)?;
            let model = self.actor_model(user)?;
            let mut model = model
                .lock()
                .map_err(|_| "MP-11: desktop actor lock unavailable")?;
            model.check_desktop_release(&actor.actor_id, surface, generation)?;
            backend.host_request_classified("host.computer.reset", serde_json::json!({}))?;
            model.release_desktop(&actor.actor_id, surface, generation)?;
            return Ok(serde_json::json!({"released":true}));
        }
        // Human keystrokes claim Desktop immediately and cancel agent input before
        // waiting on controller I/O. The shared action model reserves the same target.
        let mutation = matches!(params["op"].as_str(), Some("input" | "target_action"));
        if mutation && admission.is_none_or(|a| a.agent.is_none()) {
            let model = self.actor_model(user)?;
            let mut model = model
                .lock()
                .map_err(|_| "MP-11: desktop actor lock unavailable")?;
            if admission.is_some_and(|a| a.revoked_in_actor_lock()) {
                return Err("MP-11: not_granted: terminal desktop authority retired".into());
            }
            model.takeover_desktop(
                browser_actor(admission, &params),
                params["surface_id"].as_str().unwrap_or_default(),
                params["generation"].as_str().unwrap_or_default(),
            )?;
        }
        let backend = self.backend(user)?;
        let mut backend = backend
            .lock()
            .map_err(|_| "MP-11: desktop operation lock unavailable")?;
        self.check_admission(admission)?;
        self.require_running()?;
        let ready =
            matches!(backend.health(),Ok(h) if h.state==BrowserControllerProcessState::Ready);
        if params["op"] == "start"
            && admission
                .and_then(|a| a.agent.as_deref())
                .is_some_and(|agent| !self.is_focused(user, agent))
        {
            if !ready {
                return Err("MP-08: not_focused_agent: a new desktop requires focus".into());
            }
            let current = backend
                .host_request_classified("host.computer", serde_json::json!({"op":"state"}))?;
            self.claim_resource(
                admission,
                UserDomainResource::Desktop {
                    surface_id: current["surface_id"]
                        .as_str()
                        .ok_or("MP-11: desktop unavailable")?
                        .into(),
                },
                false,
            )?;
        }
        if !ready {
            if params["op"] != "start" {
                return Err(
                    "MP-11: browser_unavailable: explicitly start the owned desktop".into(),
                );
            }
            backend.start()?;
        }
        if let Some(a) = admission.filter(|a| a.agent.is_some()) {
            let state = self
                .inner
                .lock()
                .map_err(|_| "MP-11: desktop grant lock unavailable")?;
            Self::check_admission_epoch(&state, a)?;
            params["_agent_input"] = true.into();
            params["_subscription_owner"] = state
                .access
                .grant(user, a.agent.as_deref().unwrap())?
                .subscription_owner
                .clone()
                .into();
            params["observed_by"] = format!("agent:{}", a.agent.as_deref().unwrap()).into();
        }
        backend.host_request_classified("host.protect", policy)?;
        self.check_admission(admission)?;
        if params["op"] == "start" {
            let state = backend.host_request_cancellable(
                "host.computer",
                params.clone(),
                admission.map(|a| a.cancellation.clone()),
            )?;
            self.check_admission(admission)?;
            let surface = state["surface_id"]
                .as_str()
                .ok_or("MP-11: invalid desktop identity")?;
            self.claim_resource(
                admission,
                UserDomainResource::Desktop {
                    surface_id: surface.into(),
                },
                false,
            )?;
            let browser_state = backend
                .host_request_classified("host.browser", serde_json::json!({"op":"state"}))?;
            let model = self.actor_model(user)?;
            let mut model = model
                .lock()
                .map_err(|_| "MP-11: desktop actor lock unavailable")?;
            model.reconcile(&browser_state)?;
            model.reconcile_desktop(&state)?;
            return Ok(state);
        }
        let state = backend.host_request_cancellable(
            "host.computer",
            serde_json::json!({"op":"state"}),
            admission.map(|a| a.cancellation.clone()),
        )?;
        self.check_admission(admission)?;
        let surface = state["surface_id"]
            .as_str()
            .ok_or("MP-11: invalid desktop identity")?;
        self.claim_resource(
            admission,
            UserDomainResource::Desktop {
                surface_id: surface.into(),
            },
            false,
        )?;
        let model = self.actor_model(user)?;
        {
            let browser_state = backend
                .host_request_classified("host.browser", serde_json::json!({"op":"state"}))?;
            let mut model = model
                .lock()
                .map_err(|_| "MP-11: desktop actor lock unavailable")?;
            model.reconcile(&browser_state)?;
            model.reconcile_desktop(&state)?;
        }
        if params["op"] == "state" {
            return Ok(state);
        }
        if params["surface_id"] != state["surface_id"]
            || params["generation"] != state["generation"]
        {
            return Err("MP-11: stale native desktop".into());
        }
        let action = if mutation {
            let mut ledger = model
                .lock()
                .map_err(|_| "MP-11: desktop actor lock unavailable")?;
            if admission.is_some_and(|a| a.revoked_in_actor_lock()) {
                return Err("MP-11: not_granted: desktop authority revoked".into());
            }
            let mut metadata = params.clone();
            metadata["_native"] = true.into();
            let (id, cancellation) = ledger.begin(browser_actor(admission, &params), &metadata)?;
            Some(BrowserActorAction {
                model: model.clone(),
                id,
                cancellation,
                finished: false,
            })
        } else {
            None
        };
        let cancellation = match (&action, admission) {
            (Some(action), Some(a)) => {
                let authority = a.cancellation.clone();
                Some(Arc::new(BrowserCancellation::for_authority(
                    action.cancellation.clone(),
                    move || !authority.requested(),
                )))
            }
            (Some(action), None) => Some(action.cancellation.clone()),
            (None, Some(a)) => Some(a.cancellation.clone()),
            _ => None,
        };
        // MP-08/MP-11: a human desktop video subscription starts a viewer lease.
        let viewer = (params["op"] == "display_subscribe")
            .then(|| browser_actor(admission, &params).actor_id);
        let result =
            backend.host_request_cancellable("host.computer", params, cancellation.clone());
        if let (Some(actor), Ok(reply)) = (&viewer, &result) {
            if let Some(subscription) = reply["subscription_id"].as_str() {
                model
                    .lock()
                    .map_err(|_| "MP-11: desktop actor lock unavailable")?
                    .desktop_viewer(actor, subscription, std::time::Instant::now());
            }
        }
        if let Some(action) = action {
            let terminal = if cancellation.as_ref().is_some_and(|c| c.requested()) {
                EnvironmentActionTerminal::Cancelled
            } else if result.is_ok() {
                EnvironmentActionTerminal::Completed
            } else {
                EnvironmentActionTerminal::Failed
            };
            // Reset even persistent physical key events before cancelled/failed
            // input releases its shared Desktop target. Never replay a mutation.
            if terminal != EnvironmentActionTerminal::Completed
                && backend
                    .host_request_classified("host.computer.reset", serde_json::json!({}))
                    .is_err()
            {
                let _ = backend.stop();
            }
            action.finish(terminal);
        }
        self.check_admission(admission)?;
        if result.is_ok() {
            self.resource_notice(
                admission,
                UserDomainResource::Desktop {
                    surface_id: surface.into(),
                },
            );
        }
        result
    }
}
