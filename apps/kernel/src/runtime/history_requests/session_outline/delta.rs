//! Compare projected rows, since an event can replace content at its old sequence
//! and an append can change grouping, lifecycle or the set of rows in the page.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher, RandomState};
use std::io::{self, Write};
use std::sync::LazyLock;

use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
use serde::Serialize;

use crate::error::DaemonError;
use crate::history::HistoryEvent;
use crate::local::{
    SessionHistoryOutlineAgent, SessionHistoryOutlineDeltaCursor, SessionHistoryOutlineDeltaItem,
    SessionHistoryOutlineSummaryDelta, SessionHistoryOutlineTurnDelta,
};
use crate::session_history_page::SessionHistoryPageEntry;

static REVISIONS: LazyLock<RandomState> = LazyLock::new(RandomState::new);

pub(super) fn blob_revision(events: &[&HistoryEvent]) -> String {
    let mut hash = REVISIONS.build_hasher();
    for event in events {
        (
            event.sequence,
            event.timestamp_ms,
            &event.provider_run_id,
            &event.content,
            &event.content_ref,
            &event.metadata,
        )
            .hash(&mut hash);
    }
    format!("{:016x}", hash.finish())
}

pub(super) fn apply(
    session_id: &str,
    agent: &mut SessionHistoryOutlineAgent,
    cursor: Option<&SessionHistoryOutlineDeltaCursor>,
) -> Result<(), DaemonError> {
    let first_sequence = agent
        .turns
        .first()
        .map_or(0, |turn| turn.user_prompt.entry_index as u64);
    let after_sequence = agent
        .turns
        .iter()
        .map(|turn| {
            let last_entry = std::iter::once(&turn.user_prompt)
                .chain(&turn.summary)
                .chain(&turn.entries)
                .map(entry_sequence_end)
                .max()
                .unwrap_or_default();
            turn.blobs
                .iter()
                .map(|blob| blob.sequence_end)
                .fold(last_entry, u64::max)
        })
        .max()
        .unwrap_or_default();
    let scope = REVISIONS.hash_one((session_id, &agent.agent_id));
    let previous = cursor
        .filter(|cursor| (first_sequence..=after_sequence).contains(&cursor.after_sequence))
        .and_then(|cursor| decode_basis(scope, cursor));
    let mut basis = Basis {
        previous: previous.as_ref(),
        revisions: Vec::new(),
        summaries: Vec::new(),
    };
    let mut deltas = Vec::new();
    for turn in &mut agent.turns {
        let summary = turn.summary.take();
        let entries = std::mem::take(&mut turn.entries);
        let blobs = std::mem::take(&mut turn.blobs);
        let header = basis.record(turn)?;
        let summary_index = summary
            .as_ref()
            .map(|entry| basis.record_summary(entry))
            .transpose()?;
        let entry_indices = basis.record_all(&entries)?;
        let blob_indices = basis.record_all(&blobs)?;
        if previous.is_some() {
            deltas.push(SessionHistoryOutlineTurnDelta {
                header: retained_or_changed(header, || turn.clone()),
                summary: summary
                    .zip(summary_index)
                    .map(|(item, change)| change.apply(item)),
                entries: combine(entries, entry_indices),
                blobs: combine(blobs, blob_indices),
            });
        } else {
            turn.summary = summary;
            turn.entries = entries;
            turn.blobs = blobs;
        }
    }
    if previous.is_some() {
        agent.turns.clear();
        agent.turn_deltas = Some(deltas);
    }
    let revision = encode_basis(scope, after_sequence, &basis);
    agent.delta_cursor = Some(SessionHistoryOutlineDeltaCursor {
        after_sequence,
        revision,
    });
    Ok(())
}

fn entry_sequence_end(entry: &SessionHistoryPageEntry) -> u64 {
    entry.sequence_end.unwrap_or(entry.entry_index) as u64
}

fn retained_or_changed<T>(
    position: Option<usize>,
    value: impl FnOnce() -> T,
) -> SessionHistoryOutlineDeltaItem<T> {
    match position {
        Some(index) => SessionHistoryOutlineDeltaItem::Retained(index),
        None => SessionHistoryOutlineDeltaItem::Changed(value()),
    }
}

fn combine<T>(
    values: Vec<T>,
    positions: Vec<Option<usize>>,
) -> Vec<SessionHistoryOutlineDeltaItem<T>> {
    values
        .into_iter()
        .zip(positions)
        .map(|(item, position)| retained_or_changed(position, || item))
        .collect()
}

struct Basis<'a> {
    previous: Option<&'a PreviousBasis>,
    revisions: Vec<u64>,
    summaries: Vec<SummaryWitness>,
}

struct PreviousBasis {
    revisions: HashMap<u64, usize>,
    summaries: HashMap<usize, SummaryWitness>,
}

struct SummaryWitness {
    entry_index: usize,
    position: usize,
    text_bytes: usize,
    text_revision: u64,
}

enum SummaryChange {
    Row(Option<usize>),
    Append { position: usize, text_bytes: usize },
}

impl SummaryChange {
    fn apply(self, mut entry: SessionHistoryPageEntry) -> SessionHistoryOutlineSummaryDelta {
        match self {
            Self::Row(position) => {
                SessionHistoryOutlineSummaryDelta::Row(retained_or_changed(position, || entry))
            }
            Self::Append {
                position,
                text_bytes,
            } => {
                entry.entry.text = entry.entry.text.split_off(text_bytes);
                SessionHistoryOutlineSummaryDelta::Appended {
                    append_to: position,
                    suffix: entry,
                }
            }
        }
    }
}

impl Basis<'_> {
    fn record(&mut self, value: &impl Serialize) -> Result<Option<usize>, DaemonError> {
        let mut hash = HashWriter(REVISIONS.build_hasher());
        serde_json::to_writer(&mut hash, value).map_err(|error| DaemonError::LocalTransport {
            operation: "encode session history outline revision",
            message: error.to_string(),
        })?;
        let revision = hash.0.finish();
        self.revisions.push(revision);
        Ok(self
            .previous
            .and_then(|basis| basis.revisions.get(&revision).copied()))
    }

    fn record_summary(
        &mut self,
        entry: &SessionHistoryPageEntry,
    ) -> Result<SummaryChange, DaemonError> {
        let text = &entry.entry.text;
        self.summaries.push(SummaryWitness {
            entry_index: entry.entry_index,
            position: self.revisions.len(),
            text_bytes: text.len(),
            text_revision: REVISIONS.hash_one(text.as_bytes()),
        });
        let retained = self.record(entry)?;
        if retained.is_none() {
            if let Some(old) = self
                .previous
                .and_then(|basis| basis.summaries.get(&entry.entry_index))
            {
                // A grouped streaming reply often grows by a few words. A
                // rewrite with the same sequence is sent whole unless its
                // current text is known to contain the entire old prefix.
                if text.len() > old.text_bytes
                    && text.get(..old.text_bytes).is_some_and(|prefix| {
                        REVISIONS.hash_one(prefix.as_bytes()) == old.text_revision
                    })
                {
                    return Ok(SummaryChange::Append {
                        position: old.position,
                        text_bytes: old.text_bytes,
                    });
                }
            }
        }
        Ok(SummaryChange::Row(retained))
    }

    fn record_all<T: Serialize>(
        &mut self,
        values: &[T],
    ) -> Result<Vec<Option<usize>>, DaemonError> {
        values.iter().map(|value| self.record(value)).collect()
    }
}

struct HashWriter(std::hash::DefaultHasher);

impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encode_basis(scope: u64, sequence: u64, basis: &Basis<'_>) -> String {
    let mut bytes = Vec::new();
    bytes.extend((basis.revisions.len() as u64).to_le_bytes());
    bytes.extend(
        basis
            .revisions
            .iter()
            .flat_map(|revision| revision.to_le_bytes()),
    );
    for summary in &basis.summaries {
        bytes.extend((summary.entry_index as u64).to_le_bytes());
        bytes.extend((summary.position as u64).to_le_bytes());
        bytes.extend((summary.text_bytes as u64).to_le_bytes());
        bytes.extend(summary.text_revision.to_le_bytes());
    }
    let seal = REVISIONS.hash_one((scope, sequence, bytes.as_slice()));
    bytes.extend(seal.to_le_bytes());
    STANDARD_NO_PAD.encode(bytes)
}

fn decode_basis(scope: u64, cursor: &SessionHistoryOutlineDeltaCursor) -> Option<PreviousBasis> {
    let bytes = STANDARD_NO_PAD.decode(&cursor.revision).ok()?;
    let (payload, seal) = bytes.split_at_checked(bytes.len().checked_sub(8)?)?;
    if seal
        != REVISIONS
            .hash_one((scope, cursor.after_sequence, payload))
            .to_le_bytes()
    {
        return None;
    }
    let (words, remainder) = payload.as_chunks::<8>();
    if !remainder.is_empty() {
        return None;
    }
    let (count, words) = words.split_first()?;
    let (revisions, summaries) =
        words.split_at_checked(usize::try_from(u64::from_le_bytes(*count)).ok()?)?;
    let (summaries, remainder) = summaries.as_chunks::<4>();
    if !remainder.is_empty() {
        return None;
    }
    Some(PreviousBasis {
        revisions: revisions
            .iter()
            .enumerate()
            .map(|(position, revision)| (u64::from_le_bytes(*revision), position))
            .collect(),
        summaries: summaries
            .iter()
            .map(|words| {
                let [entry_index, position, text_bytes, text_revision] =
                    words.map(u64::from_le_bytes);
                let entry_index = entry_index as usize;
                (
                    entry_index,
                    SummaryWitness {
                        entry_index,
                        position: position as usize,
                        text_bytes: text_bytes as usize,
                        text_revision,
                    },
                )
            })
            .collect(),
    })
}
