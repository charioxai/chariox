use super::*;

impl KernelRuntimeState {
    pub(crate) fn app_control(&self) -> &crate::runtime::app_control::AppControlService {
        &self.owned.app_control
    }
}
