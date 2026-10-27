use super::*;

impl KernelRuntimeState {
    pub(crate) fn app_install_session_member(&self, session_id: &str, owner: &str) -> bool {
        self.owned
            .session_store
            .get_session(session_id)
            .is_ok_and(|session| session.has_member(owner))
    }

    pub(crate) fn app_control(&self) -> &crate::runtime::app_control::AppControlService {
        &self.owned.app_control
    }
}
