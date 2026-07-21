#[derive(Default)]
pub(super) struct WorkflowPromptDispatches {
    pub(super) local: Vec<crate::app::KernelPromptDispatch>,
    pub(super) remote: Vec<crate::app::KernelRemotePromptDispatch>,
    pub(super) starting_provider_runs: Vec<String>,
    pub(super) starting_metaagent_tasks: Vec<crate::session::QueuedMetaagentTask>,
}

impl WorkflowPromptDispatches {
    pub(super) fn extend(&mut self, other: Self) {
        self.local.extend(other.local);
        self.remote.extend(other.remote);
        self.starting_provider_runs
            .extend(other.starting_provider_runs);
        self.starting_metaagent_tasks
            .extend(other.starting_metaagent_tasks);
    }
}
