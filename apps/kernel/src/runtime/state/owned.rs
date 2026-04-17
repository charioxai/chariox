//! Owned-state result envelopes shared across runtime-state domains.
//!
//! These structs keep mutation methods explicit about which owned objects were removed or
//! completed, without exposing broader `KernelRuntimeOwnedState` internals.

pub(in crate::runtime::state) struct OwnedProviderRunExit {
    pub(in crate::runtime::state) ended_run: crate::provider::RuntimeProviderRun,
    pub(in crate::runtime::state) already_ended: bool,
}

pub(in crate::runtime::state) struct OwnedPromptCompletion {
    pub(in crate::runtime::state) completion: crate::session::PromptCompletion,
    pub(in crate::runtime::state) released_claim: bool,
    pub(in crate::runtime::state) dispatch: Option<crate::app::KernelPromptDispatch>,
}

pub(in crate::runtime::state) struct OwnedPromptCancellation {
    pub(in crate::runtime::state) cancellation: crate::session::PromptCancellation,
    pub(in crate::runtime::state) released_claim: bool,
    pub(in crate::runtime::state) dispatch: Option<crate::app::KernelPromptDispatch>,
}
