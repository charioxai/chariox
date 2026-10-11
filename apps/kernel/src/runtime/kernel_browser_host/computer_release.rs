//! Explicit human release keeps Desktop fenced until helper reset succeeds.
use super::*;
impl KernelBrowserHost {
    pub(super) fn release_computer_desktop(
        &self,
        user: &str,
        admission: Option<&KernelBrowserAdmission>,
        actor: EnvironmentActor,
        surface: &str,
        generation: &str,
    ) -> Result<Value, crate::error::HostFailure> {
        let model = self.actor_model(user)?;
        let mut model = model
            .lock()
            .map_err(|_| "MP-11: desktop actor lock unavailable")?;
        // Keep ownership until native keys are reset. Another actor cannot
        // begin input while the release waits for in-flight controller work.
        // Only the kernel authority owner can explicitly release a physical
        // local takeover through an authenticated human attachment.
        let release_owner = if model.desktop_owner_is("human:physical-owner") {
            Self::require_computer_owner(user)?;
            if admission.is_none() {
                return Err("MP-11: authenticated human desktop release required".into());
            }
            // Authority releases the physical owner's lease directly. Never
            // clear or transfer it before the helper confirms safe reset.
            "human:physical-owner"
        } else {
            &actor.actor_id
        };
        model.check_desktop_release(release_owner, surface, generation)?;
        drop(model);
        let backend = self.computer_backend(user)?;
        let mut backend = backend
            .lock()
            .map_err(|_| "MP-11: desktop backend lock unavailable")?;
        self.check_admission(admission)?;
        let model = self.actor_model(user)?;
        let model = model
            .lock()
            .map_err(|_| "MP-11: desktop actor lock unavailable")?;
        model.check_desktop_release(release_owner, surface, generation)?;
        drop(model);
        backend.request(
            "host.computer.reset",
            serde_json::json!({"physical_human_release":true}),
            None,
        )?;
        self.check_admission(admission)?;
        let model = self.actor_model(user)?;
        let mut model = model
            .lock()
            .map_err(|_| "MP-11: desktop actor lock unavailable")?;
        model.check_desktop_release(release_owner, surface, generation)?;
        model.release_desktop(release_owner, surface, generation)?;
        return Ok(serde_json::json!({"released":true}));
    }
}
