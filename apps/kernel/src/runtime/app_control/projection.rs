use super::*;
use chariox_app_runtime::installation::{Installation, ReleaseMetadata, UpdateRecord};

fn release(release: ReleaseMetadata) -> AppReleaseSummary {
    AppReleaseSummary {
        version: release.version,
        publisher_id: release.publisher_id,
        package_digest: release.package_digest,
        schema_version: release.schema_version,
    }
}

pub(super) fn installation(value: Installation) -> AppInstallationSummary {
    AppInstallationSummary {
        installation_id: value.installation_id,
        app_id: value.app_id,
        generation: value.generation.to_string(),
        active_release: value.active.map(|active| release(active.release)),
        pending_generation: value
            .pending_generation
            .map(|generation| generation.to_string()),
        admission_paused: value.admission_paused,
    }
}

pub(super) fn update(value: UpdateRecord) -> AppUpdateSummary {
    AppUpdateSummary {
        base_generation: value.token.base_generation.to_string(),
        generation: value.token.generation.to_string(),
        release: release(value.release),
        phase: match value.phase {
            UpdatePhase::Staged => AppUpdatePhase::Staged,
            UpdatePhase::Quiescing => AppUpdatePhase::Quiescing,
            UpdatePhase::Prepared => AppUpdatePhase::Prepared,
            UpdatePhase::Committed => AppUpdatePhase::Committed,
            UpdatePhase::Aborted => AppUpdatePhase::Aborted,
        },
        decision: match value.decision {
            CapabilityDecision::Pending => AppCapabilityDecisionStatus::Pending,
            CapabilityDecision::Approved { .. } => AppCapabilityDecisionStatus::Approved,
            CapabilityDecision::Declined { .. } => AppCapabilityDecisionStatus::Declined,
        },
        created_at_ms: value.created_at_ms,
        updated_at_ms: value.updated_at_ms,
    }
}
