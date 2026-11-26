//! Row 582: the kernel's hold on the install of its owner's logins on a machine it acquired.
use super::*;
use crate::runtime::agent_machine_install::{InstallAttempts, InstallStarter};

impl KernelRuntimeState {
    pub(crate) fn agent_install_attempts(&self) -> &InstallAttempts {
        &self.owned.agent_install_attempts
    }

    pub(crate) fn agent_install_starter(&self) -> Option<InstallStarter> {
        self.owned.agent_install_starter.get().cloned()
    }

    /// Set once when the kernel starts serving; a later call changes nothing.
    pub(crate) fn set_agent_install_starter(&self, starter: InstallStarter) {
        let _ = self.owned.agent_install_starter.set(starter);
    }
}
