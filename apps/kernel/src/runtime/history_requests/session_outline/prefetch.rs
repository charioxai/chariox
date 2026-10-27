//! MP-08 / MP-10: bounded cross-agent content in the initial outline round trip.
use super::*;
use crate::local::SessionHistoryOutlineBlob;

const MAX_PREFETCH_BYTES: usize = 1024 * 1024;

pub(super) fn prefetch_outline_blobs(
    store: &OperationalHistoryStore,
    session_id: &str,
    agents: &mut [SessionHistoryOutlineAgent],
    imports: &BTreeMap<String, ExternalProviderImportMetadata>,
) -> Result<(), DaemonError> {
    let mut remaining_bytes = MAX_PREFETCH_BYTES;
    for agent in agents {
        let blobs = selected_blobs(agent);
        let mut pages = Vec::new();
        // One indexed read per agent, using the outline's exact ranges.
        if let (Some(start), Some(end)) = (
            blobs.iter().map(|blob| blob.sequence_start).min(),
            blobs.iter().map(|blob| blob.sequence_end).max(),
        ) {
            let events = store.load_session_events_for_agent_sequence_range(
                session_id,
                &agent.agent_id,
                start,
                end,
            )?;
            for blob in blobs {
                let entries = outline_blob_content_events(
                    events
                        .iter()
                        .filter(|event| {
                            event.sequence >= blob.sequence_start
                                && event.sequence <= blob.sequence_end
                        })
                        .cloned()
                        .collect(),
                )
                .into_iter()
                .filter(|event| {
                    event_belongs_to_external_import(imports.get(&agent.agent_id), event)
                })
                .filter_map(page_entry_from_event)
                .collect();
                let page = SessionHistoryBlobContentPage {
                    blob_id: blob.blob_id.clone(),
                    entries: crate::session_history_page::coalesce_history_page_entries(entries),
                };
                let size = serde_json::to_vec(&page)
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "encode prefetched history blob",
                        message: error.to_string(),
                    })?
                    .len();
                if size <= remaining_bytes {
                    remaining_bytes -= size;
                    pages.push(page);
                }
            }
        }
        // Some(empty) acknowledges the request even when the budget is spent.
        agent.prefetched_blobs = Some(pages);
    }
    Ok(())
}

fn selected_blobs(agent: &SessionHistoryOutlineAgent) -> Vec<&SessionHistoryOutlineBlob> {
    let mut selected = Vec::new();
    let mut ids = BTreeSet::new();
    for (index, turn) in agent.turns.iter().rev().enumerate() {
        for blob in &turn.blobs {
            let latest_detail = index == 0
                && (blob.kind != crate::history::SessionHistoryEntryKind::UserPrompt
                    || blob.steering);
            if (latest_detail
                || blob.kind == crate::history::SessionHistoryEntryKind::ProviderOutput
                || blob.steering)
                && ids.insert(&blob.blob_id)
            {
                selected.push(blob);
                if selected.len() == MAX_BLOB_CONTENTS_PER_REQUEST {
                    return selected;
                }
            }
        }
    }
    selected
}
