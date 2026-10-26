//! MP-08 / MP-10: compact complete owned text deltas at the response boundary.
use super::SessionHistoryPageEntry;
use crate::history::{SessionHistoryEntry, SessionHistoryEntryKind};

pub(crate) fn coalesce_history_page_entries(
    entries: Vec<SessionHistoryPageEntry>,
) -> Vec<SessionHistoryPageEntry> {
    let mut compact: Vec<SessionHistoryPageEntry> = Vec::new();
    for next in entries {
        if let Some(last) = compact.last_mut().filter(|last| can_join(last, &next)) {
            if last.sequence_byte_lengths.is_empty() {
                last.sequence_byte_lengths.push(last.entry.text.len());
                last.sequence_indices.push(last.entry_index);
            }
            last.sequence_byte_lengths.push(next.entry.text.len());
            last.sequence_indices.push(next.entry_index);
            last.sequence_end = Some(next.entry_index);
            last.entry.text.push_str(&next.entry.text);
            last.fragment_end += next.fragment_end;
            last.total_chars += next.total_chars;
        } else {
            compact.push(next);
        }
    }
    compact
}

fn can_join(last: &SessionHistoryPageEntry, next: &SessionHistoryPageEntry) -> bool {
    matches!(
        last.entry.kind,
        SessionHistoryEntryKind::ProviderOutput | SessionHistoryEntryKind::ProviderReasoning
    ) && !last.entry.is_external_provider_observed()
        && last.entry.merge_key.is_some()
        && last.sequence_end.unwrap_or(last.entry_index) < next.entry_index
        && next.sequence_end.is_none()
        && next.sequence_byte_lengths.is_empty()
        && last.fragment_start == 0
        && last.fragment_end == last.total_chars
        && next.fragment_start == 0
        && next.fragment_end == next.total_chars
        && same_metadata(&last.entry, &next.entry)
}

fn same_metadata(left: &SessionHistoryEntry, right: &SessionHistoryEntry) -> bool {
    // Timestamp follows the existing terminal-record coalescer: retain the first row's time.
    left.session_id == right.session_id
        && left.provider_run_id == right.provider_run_id
        && left.agent_id == right.agent_id
        && left.source_attachment_id == right.source_attachment_id
        && left.prompt_origin == right.prompt_origin
        && left.kind == right.kind
        && left.merge_key == right.merge_key
        && left.source == right.source
        && left.external_provider == right.external_provider
        && left.external_provider_session_id == right.external_provider_session_id
        && left.external_provider_turn_id == right.external_provider_turn_id
        && left.observed_at_ms == right.observed_at_ms
        && left.external_observation == right.external_observation
        && left.attachments == right.attachments
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::TerminalOutputKind;

    fn page(sequence: usize, text: &str) -> SessionHistoryPageEntry {
        SessionHistoryPageEntry {
            entry_index: sequence,
            sequence_end: None,
            sequence_byte_lengths: Vec::new(),
            sequence_indices: Vec::new(),
            fragment_start: 0,
            fragment_end: text.chars().count(),
            total_chars: text.chars().count(),
            entry: SessionHistoryEntry::provider_output(
                "s",
                "r",
                Some("a"),
                TerminalOutputKind::ProviderOutput,
                Some("m".into()),
                text.to_string(),
            ),
        }
    }

    #[test]
    fn unicode_deltas_keep_exact_row_boundaries() {
        let rows =
            coalesce_history_page_entries(vec![page(10, "é"), page(11, "中"), page(12, "é")]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].entry.text, "é中é");
        assert_eq!(rows[0].sequence_end, Some(12));
        assert_eq!(rows[0].sequence_byte_lengths, vec![2, 3, 2]);
        assert_eq!(rows[0].total_chars, 3);
    }

    #[test]
    fn partial_rows_and_scope_changes_never_join() {
        let mut partial = page(12, "a");
        partial.total_chars = 2;
        let mut other = page(14, "c");
        other.entry.agent_id = Some("b".into());
        let rows = vec![page(10, "a"), partial, page(13, "b"), other];
        assert_eq!(coalesce_history_page_entries(rows.clone()), rows);
    }

    #[test]
    fn interleaved_agent_sequences_keep_sparse_row_identities() {
        let rows =
            coalesce_history_page_entries(vec![page(10, "é"), page(12, "中"), page(13, "é")]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sequence_indices, vec![10, 12, 13]);
        assert_eq!(rows[0].sequence_end, Some(13));
        assert_eq!(rows[0].entry.text, "é中é");
    }

    #[test]
    fn snapshot_tools_and_observed_output_never_join() {
        let mut tool = page(10, "{}");
        tool.entry.kind = SessionHistoryEntryKind::ProviderTool;
        let mut tool2 = tool.clone();
        tool2.entry_index = 11;
        let mut observed = page(12, "a");
        observed.entry.source =
            Some(crate::history::SessionHistoryEntrySource::ExternalProviderObserved);
        let mut observed2 = observed.clone();
        observed2.entry_index = 13;
        let rows = vec![tool, tool2, observed, observed2];
        assert_eq!(coalesce_history_page_entries(rows.clone()), rows);
    }
}
