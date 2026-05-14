#[derive(Default)]
pub(super) struct WorkflowPromptDispatches {
    pub(super) local: Vec<crate::app::KernelPromptDispatch>,
    pub(super) remote: Vec<crate::app::KernelRemotePromptDispatch>,
}

impl WorkflowPromptDispatches {
    pub(super) fn extend(&mut self, other: Self) {
        self.local.extend(other.local);
        self.remote.extend(other.remote);
    }
}
