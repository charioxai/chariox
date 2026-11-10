use super::*;
use chariox_app_runtime::installation::{Installation, ReleaseMetadata, UpdateRecord};

fn release(release: ReleaseMetadata) -> AppReleaseSummary {
    AppReleaseSummary {
        version: release.version,
        publisher_id: release.publisher_id,
        package_digest: release.package_digest,
        schema_version: release.schema_version,
        display_name: release.display_name,
        icon: release.icon,
    }
}

pub(super) fn installation(value: Installation) -> AppInstallationSummary {
    let data_kept = value.active.is_none() && value.retained.is_some();
    // An uninstalled installation keeps showing the App its data belongs to.
    let (display_name, icon) = value
        .data_release()
        .map(|data| (data.release.display_name.clone(), data.release.icon.clone()))
        .unwrap_or_default();
    AppInstallationSummary {
        channel: value.channel,
        dev_room_id: value.dev_room_id,
        installation_id: value.installation_id,
        app_id: value.app_id,
        generation: value.generation.to_string(),
        active_release: value.active.map(|active| release(active.release)),
        display_name,
        icon,
        pending_generation: value
            .pending_generation
            .map(|generation| generation.to_string()),
        admission_paused: value.admission_paused,
        data_kept,
        events: Vec::new(),
    }
}

/// The installation with the outgoing events of its active release, when it has one.
pub(crate) fn installation_with_events(
    store: &DurableKernelStateStore,
    owner: &str,
    value: Installation,
) -> AppInstallationSummary {
    let events = store
        .active_app_event_catalog(owner, &value.installation_id)
        .map(|catalog| {
            catalog
                .outgoing_events()
                .map(|(name, title)| AppEventSummary {
                    name: name.to_owned(),
                    label: title.unwrap_or(name).to_owned(),
                })
                .collect()
        })
        .unwrap_or_default();
    AppInstallationSummary {
        events,
        ..installation(value)
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
