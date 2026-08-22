#[derive(Default)]
pub(super) struct WorkflowPromptDispatches {
    pub(super) local: Vec<crate::app::KernelPromptDispatch>,
    pub(super) remote: Vec<crate::app::KernelRemotePromptDispatch>,
    pub(super) starting_provider_runs: Vec<String>,
    pub(super) provider_run_retirements: std::collections::BTreeMap<String, Vec<String>>,
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
            && self.provider_run_retirements.is_empty()
            && self.starting_metaagent_tasks.is_empty()
    }

    pub(super) fn extend(&mut self, other: Self) {
        self.local.extend(other.local);
        self.remote.extend(other.remote);
        self.starting_provider_runs
            .extend(other.starting_provider_runs);
        for (replacement_provider_run_id, retired_provider_run_ids) in
            other.provider_run_retirements
        {
            self.provider_run_retirements
                .entry(replacement_provider_run_id)
                .or_default()
                .extend(retired_provider_run_ids);
        }
        self.starting_metaagent_tasks
            .extend(other.starting_metaagent_tasks);
        self.admitted_workflow_prompt |= other.admitted_workflow_prompt;
    }

    pub(super) fn mark_workflow_prompt_admitted(&mut self) {
        self.admitted_workflow_prompt = true;
    }

    pub(super) fn retire_provider_before_launch(
        &mut self,
        replacement_provider_run_id: impl Into<String>,
        retired_provider_run_id: impl Into<String>,
    ) {
        self.provider_run_retirements
            .entry(replacement_provider_run_id.into())
            .or_default()
            .push(retired_provider_run_id.into());
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
        let mut retiring = WorkflowPromptDispatches::default();
        retiring.retire_provider_before_launch("provider-run-new", "provider-run-old");

        assert!(!retiring.is_empty());
    }

    #[test]
    fn provider_retirements_remain_paired_with_their_replacements() {
        let mut first = WorkflowPromptDispatches::default();
        first.retire_provider_before_launch("provider-run-new-a", "provider-run-old-a");
        let mut second = WorkflowPromptDispatches::default();
        second.retire_provider_before_launch("provider-run-new-b", "provider-run-old-b");

        first.extend(second);

        assert_eq!(
            first.provider_run_retirements,
            std::collections::BTreeMap::from([
                (
                    "provider-run-new-a".to_string(),
                    vec!["provider-run-old-a".to_string()],
                ),
                (
                    "provider-run-new-b".to_string(),
                    vec!["provider-run-old-b".to_string()],
                ),
            ])
        );
    }
}
