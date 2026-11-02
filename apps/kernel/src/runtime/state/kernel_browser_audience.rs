//! MP-08 / MP-10 / MP-11: the home kernel chooses one shared Browser display policy.
use super::kernel_browser_runtime::host_error;
use super::*;
use crate::runtime::kernel_browser_host::KernelBrowserHost;
use serde_json::Value;

impl KernelRuntimeState {
    pub(super) fn kernel_browser_display_masked(&self, owner: &str) -> Result<bool, DaemonError> {
        let sessions = self.owned.session_store.list_all_sessions();
        let attachments = self.owned.attachment_store.read();
        for session in sessions {
            if self.provider_account_authority_owner_user_id(session.owner_user_id()) != owner {
                continue;
            }
            for id in attachments.list_session_attachment_ids(session.id()) {
                let attachment = attachments.get_attachment(&id)?;
                if self.provider_account_authority_owner_user_id(attachment.owner_user_id())
                    != owner
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    pub(super) fn kernel_browser_policy_revision(&self, user: &str) -> Result<u64, DaemonError> {
        let revision = self
            .owned
            .kernel_browser_secret_observations
            .revision(&KernelBrowserHost::profile_key(user))?;
        let masked = self.kernel_browser_display_masked(user)?;
        revision
            .checked_mul(2)
            .and_then(|revision| revision.checked_add(u64::from(masked)))
            .ok_or_else(|| host_error("MD-5: browser observation revision unavailable".into()))
    }

    pub(super) fn kernel_browser_capture_policy(&self, user: &str) -> Result<Value, DaemonError> {
        let scope = KernelBrowserHost::profile_key(user);
        let mut policy: Value = serde_json::from_str(
            &self
                .owned
                .kernel_browser_secret_observations
                .capture_policy(&scope)?,
        )
        .map_err(|_| host_error("MD-5: invalid observation policy".into()))?;
        // This private controller flag is never taken from a client or an agent.
        // Model captures use the full policy regardless of the display audience.
        policy["display_masked"] = self.kernel_browser_display_masked(user)?.into();
        Ok(policy)
    }

    pub(super) async fn refresh_kernel_browser_audience(
        &self,
        attachment: &crate::attachment::RuntimeAttachment,
    ) -> Result<(), DaemonError> {
        let session = self
            .owned
            .session_store
            .get_session(attachment.session_id())?;
        let user = self.provider_account_authority_owner_user_id(session.owner_user_id());
        if self.provider_account_authority_owner_user_id(attachment.owner_user_id()) == user {
            return Ok(());
        }
        let scope = KernelBrowserHost::profile_key(&user);
        let protection = &self.owned.kernel_browser_secret_observations;
        let _barrier = protection.barrier(&scope)?.write_owned().await;
        protection.require(&scope, false)?;
        let policy = self.kernel_browser_capture_policy(&user)?;
        let host = self.owned.kernel_browser_host.clone();
        tokio::task::spawn_blocking(move || host.update_protection_if_running(&user, policy))
            .await
            .map_err(|_| host_error("MD-5: display audience update interrupted".into()))?
            .map_err(host_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::runtime::state::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;

    #[tokio::test]
    async fn mp08_mp11_browser_audience_follows_authenticated_attachments() {
        let room = TestRoom::new("browser-audience");
        let state = &room.runtime;
        let owner = state
            .owned
            .session_store
            .get_session(&room.session_id)
            .unwrap()
            .owner_user_id()
            .to_owned();
        let owner = state.provider_account_authority_owner_user_id(&owner);
        assert!(!state.kernel_browser_display_masked(&owner).unwrap());
        let attach = |user: &str, client: &str| {
            state
                .owned
                .attach(AttachRequest::for_user(
                    &room.session_id,
                    client,
                    ClientCapabilityLevel::FullTerminal,
                    user,
                ))
                .unwrap()
        };
        let own = attach(&owner, "owner");
        assert!(!state.kernel_browser_display_masked(&owner).unwrap());
        let before = state.kernel_browser_policy_revision(&owner).unwrap();
        let foreign = attach("invited-person", "viewer");
        assert_ne!(
            state.kernel_browser_policy_revision(&owner).unwrap(),
            before,
            "a cached owner packet cannot replay while a non-owner watches"
        );
        assert!(state.kernel_browser_display_masked(&owner).unwrap());
        state.owned.detach(foreign.id()).unwrap();
        assert!(!state.kernel_browser_display_masked(&owner).unwrap());
        state.owned.detach(own.id()).unwrap();
    }
}
