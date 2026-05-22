#[derive(Default)]
pub(super) struct WorkflowPromptDispatches {
    pub(super) local: Vec<crate::app::KernelPromptDispatch>,
    pub(super) remote: Vec<crate::app::KernelRemotePromptDispatch>,
    pub(super) starting_provider_runs: Vec<String>,
}

impl WorkflowPromptDispatches {
    pub(super) fn extend(&mut self, other: Self) {
        self.local.extend(other.local);
        self.remote.extend(other.remote);
        self.starting_provider_runs
            .extend(other.starting_provider_runs);
    }
}
