#[derive(Default)]
pub(super) struct WorkflowPromptDispatches {
    pub(super) local: Vec<crate::app::KernelPromptDispatch>,
    pub(super) remote: Vec<crate::app::KernelRemotePromptDispatch>,
    pub(super) starting_provider_runs: Vec<String>,
    pub(super) retiring_provider_runs: Vec<String>,
    pub(super) starting_metaagent_tasks: Vec<crate::session::QueuedMetaagentTask>,
    /// A prompt may be admitted to a busy provider queue without producing a dispatch yet.
    /// Keep this admission signal separate from the concrete work vectors.
    pub(super) admitted_workflow_prompt: bool,
}

impl WorkflowPromptDispatches {
    pub(super) fn is_empty(&self) -> bool {
        self.local.is_empty()
            && self.remote.is_empty()
            && self.starting_provider_runs.is_empty()
            && self.retiring_provider_runs.is_empty()
            && self.starting_metaagent_tasks.is_empty()
    }

    pub(super) fn extend(&mut self, other: Self) {
        self.local.extend(other.local);
        self.remote.extend(other.remote);
        self.starting_provider_runs
            .extend(other.starting_provider_runs);
        self.retiring_provider_runs
            .extend(other.retiring_provider_runs);
        self.starting_metaagent_tasks
            .extend(other.starting_metaagent_tasks);
        self.admitted_workflow_prompt |= other.admitted_workflow_prompt;
    }

    pub(super) fn mark_workflow_prompt_admitted(&mut self) {
        self.admitted_workflow_prompt = true;
    }
}

#[cfg(test)]
mod tests {
    use super::WorkflowPromptDispatches;

    #[test]
    fn queued_admission_is_distinct_from_concrete_dispatches() {
        let mut admitted = WorkflowPromptDispatches::default();
        admitted.mark_workflow_prompt_admitted();

        assert!(admitted.is_empty());

        let mut combined = WorkflowPromptDispatches::default();
        combined.extend(admitted);

        assert!(combined.is_empty());
        assert!(combined.admitted_workflow_prompt);
    }

    #[test]
    fn provider_retirement_is_dispatch_work() {
        let retiring = WorkflowPromptDispatches {
            retiring_provider_runs: vec!["provider-run-old".to_string()],
            ..WorkflowPromptDispatches::default()
        };

        assert!(!retiring.is_empty());
    }
}
