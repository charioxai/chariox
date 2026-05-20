use serde::{Deserialize, Serialize};

use crate::history::{SessionHistoryEntry, SessionHistoryEntryKind};

const DEFAULT_SESSION_HISTORY_ROUND_COUNT: usize = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHistoryCursor {
    pub before_entry_index: usize,
    pub before_entry_char_offset: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHistoryPageEntry {
    pub entry_index: usize,
    pub fragment_start: usize,
    pub fragment_end: usize,
    pub total_chars: usize,
    pub entry: SessionHistoryEntry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHistoryPage {
    pub entries: Vec<SessionHistoryPageEntry>,
    pub next_cursor: Option<SessionHistoryCursor>,
}

#[derive(Debug, Clone)]
struct SessionHistorySlice {
    entry_index: usize,
    fragment_start: usize,
    fragment_end: usize,
    total_chars: usize,
    entry: SessionHistoryEntry,
}

pub fn paginate_session_history(
    entries: &[SessionHistoryEntry],
    round_count: Option<usize>,
    max_chars: Option<usize>,
    before_entry_index: Option<usize>,
    before_entry_char_offset: Option<usize>,
) -> SessionHistoryPage {
    paginate_session_history_from_index(
        entries,
        0,
        round_count,
        max_chars,
        before_entry_index,
        before_entry_char_offset,
    )
}

pub(crate) fn paginate_session_history_from_index(
    entries: &[SessionHistoryEntry],
    base_entry_index: usize,
    round_count: Option<usize>,
    max_chars: Option<usize>,
    before_entry_index: Option<usize>,
    before_entry_char_offset: Option<usize>,
) -> SessionHistoryPage {
    let mut slices = build_history_slices(
        entries,
        base_entry_index,
        before_entry_index,
        before_entry_char_offset,
    );

    if slices.is_empty() {
        return SessionHistoryPage {
            entries: Vec::new(),
            next_cursor: None,
        };
    }

    let mut start_index = history_start_for_recent_user_rounds_in_slices(
        &slices,
        round_count.unwrap_or(DEFAULT_SESSION_HISTORY_ROUND_COUNT),
    );

    if let Some(max_chars) = max_chars {
        start_index = trim_history_slices_to_budget(&mut slices, start_index, max_chars);
    }

    let page_slices = slices.split_off(start_index);
    let next_cursor = page_slices.first().and_then(next_history_cursor_for_slice);

    SessionHistoryPage {
        entries: page_slices
            .into_iter()
            .map(|slice| SessionHistoryPageEntry {
                entry_index: slice.entry_index,
                fragment_start: slice.fragment_start,
                fragment_end: slice.fragment_end,
                total_chars: slice.total_chars,
                entry: slice.entry,
            })
            .collect(),
        next_cursor,
    }
}

#[cfg(test)]
pub fn history_start_for_recent_user_rounds(
    entries: &[SessionHistoryEntry],
    round_count: usize,
) -> usize {
    if round_count == 0 || entries.is_empty() {
        return 0;
    }

    let mut seen_user_prompts = 0usize;
    for (index, entry) in entries.iter().enumerate().rev() {
        if entry.kind == SessionHistoryEntryKind::UserPrompt {
            seen_user_prompts += 1;
            if seen_user_prompts == round_count {
                return index;
            }
        }
    }

    0
}

fn history_start_for_recent_user_rounds_in_slices(
    entries: &[SessionHistorySlice],
    round_count: usize,
) -> usize {
    if round_count == 0 || entries.is_empty() {
        return 0;
    }

    let mut seen_user_prompts = 0usize;
    for (index, entry) in entries.iter().enumerate().rev() {
        if entry.entry.kind == SessionHistoryEntryKind::UserPrompt {
            seen_user_prompts += 1;
            if seen_user_prompts == round_count {
                return index;
            }
        }
    }

    0
}

fn build_history_slices(
    entries: &[SessionHistoryEntry],
    base_entry_index: usize,
    before_entry_index: Option<usize>,
    before_entry_char_offset: Option<usize>,
) -> Vec<SessionHistorySlice> {
    let end_entry_index = base_entry_index.saturating_add(entries.len());
    let before_entry_index = before_entry_index
        .unwrap_or(end_entry_index)
        .min(end_entry_index);
    let retained_entry_count = before_entry_index
        .saturating_sub(base_entry_index)
        .min(entries.len());
    let mut slices: Vec<SessionHistorySlice> = entries
        .iter()
        .cloned()
        .take(retained_entry_count)
        .enumerate()
        .map(|(entry_index, entry)| {
            let total_chars = entry.text.chars().count();
            SessionHistorySlice {
                entry_index: base_entry_index + entry_index,
                fragment_start: 0,
                fragment_end: total_chars,
                total_chars,
                entry,
            }
        })
        .collect();

    if let Some(slice) = slices.last_mut() {
        if let Some(before_entry_char_offset) = before_entry_char_offset {
            let fragment_end = before_entry_char_offset.min(slice.fragment_end);
            slice.fragment_end = fragment_end;
            slice.entry.text = text_prefix(&slice.entry.text, fragment_end);
        }
    }

    while matches!(slices.last(), Some(slice) if slice.fragment_end == 0) {
        slices.pop();
    }

    slices
}

fn trim_history_slices_to_budget(
    slices: &mut [SessionHistorySlice],
    mut start_index: usize,
    max_chars: usize,
) -> usize {
    let mut total_chars: usize = slices[start_index..]
        .iter()
        .map(|slice| slice.fragment_end.saturating_sub(slice.fragment_start))
        .sum();

    while total_chars > max_chars && start_index < slices.len() {
        let slice = &mut slices[start_index];
        let slice_chars = slice.fragment_end.saturating_sub(slice.fragment_start);
        let overflow = total_chars - max_chars;
        if slice_chars > overflow {
            slice.fragment_start += overflow;
            slice.entry.text = text_range(&slice.entry.text, overflow, slice_chars);
            break;
        }
        total_chars -= slice_chars;
        start_index += 1;
    }

    start_index
}

fn next_history_cursor_for_slice(slice: &SessionHistorySlice) -> Option<SessionHistoryCursor> {
    if slice.fragment_start > 0 {
        return Some(SessionHistoryCursor {
            before_entry_index: slice.entry_index + 1,
            before_entry_char_offset: Some(slice.fragment_start),
        });
    }
    if slice.entry_index > 0 {
        return Some(SessionHistoryCursor {
            before_entry_index: slice.entry_index,
            before_entry_char_offset: None,
        });
    }
    None
}

fn text_prefix(text: &str, char_count: usize) -> String {
    text.chars().take(char_count).collect()
}

fn text_range(text: &str, start: usize, char_count: usize) -> String {
    text.chars().skip(start).take(char_count - start).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        history_start_for_recent_user_rounds, paginate_session_history,
        paginate_session_history_from_index, SessionHistoryCursor, SessionHistoryPage,
        SessionHistoryPageEntry,
    };
    use crate::history::{SessionHistoryEntry, SessionHistoryEntryKind};

    #[test]
    fn preserves_four_recent_user_rounds_when_trimming_history() {
        let entries = vec![
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 1"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 1"),
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 2"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 2"),
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 3"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 3"),
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 4"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 4"),
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 5"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 5"),
        ];

        assert_eq!(history_start_for_recent_user_rounds(&entries, 4), 2);
    }

    #[test]
    fn returns_next_before_index_when_older_rounds_exist() {
        let entries = vec![
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 1"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 1"),
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 2"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 2"),
        ];

        let page = page_for_rounds(&entries, 1, None, None, None);

        assert_eq!(
            page,
            SessionHistoryPage {
                entries: vec![
                    page_entry(
                        2,
                        0,
                        8,
                        history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 2")
                    ),
                    page_entry(
                        3,
                        0,
                        8,
                        history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 2")
                    ),
                ],
                next_cursor: Some(SessionHistoryCursor {
                    before_entry_index: 2,
                    before_entry_char_offset: None,
                }),
            }
        );
    }

    #[test]
    fn pages_history_from_the_previous_cursor() {
        let entries = vec![
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 1"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 1"),
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 2"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 2"),
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 3"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 3"),
        ];

        let page = page_for_rounds(&entries, 1, None, Some(4), None);

        assert_eq!(
            page,
            SessionHistoryPage {
                entries: vec![
                    page_entry(
                        2,
                        0,
                        8,
                        history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 2")
                    ),
                    page_entry(
                        3,
                        0,
                        8,
                        history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 2")
                    ),
                ],
                next_cursor: Some(SessionHistoryCursor {
                    before_entry_index: 2,
                    before_entry_char_offset: None,
                }),
            }
        );
    }

    #[test]
    fn slices_large_history_entries_without_truncation() {
        let page = page_for_rounds(
            &[
                history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 1"),
                history_entry(SessionHistoryEntryKind::ProviderOutput, &"x".repeat(24)),
            ],
            1,
            Some(10),
            None,
            None,
        );

        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].entry.text, "xxxxxxxxxx");
        assert_eq!(page.entries[0].fragment_start, 14);
        assert_eq!(page.entries[0].fragment_end, 24);
        assert_eq!(
            page.next_cursor,
            Some(SessionHistoryCursor {
                before_entry_index: 2,
                before_entry_char_offset: Some(14),
            })
        );
    }

    #[test]
    fn continues_loading_the_older_part_of_a_partial_entry() {
        let entries = vec![
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 1"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, &"x".repeat(24)),
        ];

        let page = page_for_rounds(&entries, 1, Some(10), Some(2), Some(14));

        assert_eq!(
            page,
            SessionHistoryPage {
                entries: vec![page_entry(
                    1,
                    4,
                    24,
                    history_entry(SessionHistoryEntryKind::ProviderOutput, "xxxxxxxxxx"),
                )],
                next_cursor: Some(SessionHistoryCursor {
                    before_entry_index: 2,
                    before_entry_char_offset: Some(4),
                }),
            }
        );
    }

    #[test]
    fn preserves_absolute_entry_indices_for_retained_suffixes() {
        let entries = vec![
            history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 3"),
            history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 3"),
        ];

        let page = paginate_session_history_from_index(&entries, 4, Some(1), None, None, None);

        assert_eq!(
            page,
            SessionHistoryPage {
                entries: vec![
                    page_entry(
                        4,
                        0,
                        8,
                        history_entry(SessionHistoryEntryKind::UserPrompt, "prompt 3")
                    ),
                    page_entry(
                        5,
                        0,
                        8,
                        history_entry(SessionHistoryEntryKind::ProviderOutput, "answer 3")
                    ),
                ],
                next_cursor: Some(SessionHistoryCursor {
                    before_entry_index: 4,
                    before_entry_char_offset: None,
                }),
            }
        );
    }

    fn page_for_rounds(
        entries: &[SessionHistoryEntry],
        round_count: usize,
        max_chars: Option<usize>,
        before_entry_index: Option<usize>,
        before_entry_char_offset: Option<usize>,
    ) -> SessionHistoryPage {
        paginate_session_history(
            entries,
            Some(round_count),
            max_chars,
            before_entry_index,
            before_entry_char_offset,
        )
    }

    fn page_entry(
        entry_index: usize,
        fragment_start: usize,
        total_chars: usize,
        entry: SessionHistoryEntry,
    ) -> SessionHistoryPageEntry {
        SessionHistoryPageEntry {
            entry_index,
            fragment_start,
            fragment_end: fragment_start + entry.text.chars().count(),
            total_chars,
            entry,
        }
    }

    fn history_entry(kind: SessionHistoryEntryKind, text: &str) -> SessionHistoryEntry {
        SessionHistoryEntry {
            session_id: "session-1".to_string(),
            provider_run_id: Some("run-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            source_attachment_id: Some("attachment-1".to_string()),
            kind,
            merge_key: None,
            text: text.to_string(),
            timestamp_ms: 0,
        }
    }
}
