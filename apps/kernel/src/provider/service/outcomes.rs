use crate::provider::RuntimeProviderRun;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderRunLivenessReconciliation {
    AlreadyEnded(RuntimeProviderRun),
    ExternalEndpoint(RuntimeProviderRun),
    StillRunning(RuntimeProviderRun),
    NewlyEnded(RuntimeProviderRun),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderRunEndedOutcome {
    pub(super) run: RuntimeProviderRun,
    pub(super) already_ended: bool,
}

impl ProviderRunEndedOutcome {
    pub(crate) fn run(&self) -> &RuntimeProviderRun {
        &self.run
    }

    pub(crate) fn into_run(self) -> RuntimeProviderRun {
        self.run
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderSessionRunsTerminatedOutcome {
    pub(super) runs: Vec<ProviderRunEndedOutcome>,
}

impl ProviderSessionRunsTerminatedOutcome {
    pub(crate) fn runs(&self) -> &[ProviderRunEndedOutcome] {
        &self.runs
    }

    pub(crate) fn into_runs(self) -> Vec<ProviderRunEndedOutcome> {
        self.runs
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderRunStartedOutcome {
    pub(super) run: RuntimeProviderRun,
}

impl ProviderRunStartedOutcome {
    pub(crate) fn run(&self) -> &RuntimeProviderRun {
        &self.run
    }

    pub(crate) fn into_run(self) -> RuntimeProviderRun {
        self.run
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderRunParkedOutcome {
    pub(super) run: RuntimeProviderRun,
}

impl ProviderRunParkedOutcome {
    pub(crate) fn run(&self) -> &RuntimeProviderRun {
        &self.run
    }

    pub(crate) fn into_run(self) -> RuntimeProviderRun {
        self.run
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderRunResumedOutcome {
    pub(super) run: RuntimeProviderRun,
}

impl ProviderRunResumedOutcome {
    pub(crate) fn run(&self) -> &RuntimeProviderRun {
        &self.run
    }

    pub(crate) fn into_run(self) -> RuntimeProviderRun {
        self.run
    }
}
