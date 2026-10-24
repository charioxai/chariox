//! MP-08/MP-10/MP-11: streaming redaction of inserted Vault values.
//! Public text stays; only the secret bytes (and their known encodings) are replaced,
//! even when a value is split across chunks or interleaved with terminal escapes.
use std::collections::BTreeMap;

use zeroize::Zeroizing;

use super::{secret_variants, Protection, RoomSecretObservations};
use crate::runtime::state::prompt_transcript_owned_state::TerminalOutputBatchAppend as Output;
use crate::terminal::TerminalOutputKind;

const REDACTED: &[u8] = b"[redacted]";
const WITHHELD: &[u8] = b"[sensitive Room terminal stream withheld]";
const OSC_LIMIT: usize = 4096;
const CSI_LIMIT: usize = 256;
// A tail this short cannot meaningfully reveal a value, so it is not worth a marker.
const SHORT_TAIL: usize = 2;

/// Bytes held back from a stream because they may begin a secret that continues
/// in the next chunk. Keyed by room, provider run, then the output stream.
pub(super) type Pending = BTreeMap<(String, String, String), (Output, Zeroizing<Vec<u8>>)>;

enum Escape {
    None,
    // `payload` is the text carried by a string escape (OSC, DCS, ...), which may hold a value.
    Complete {
        len: usize,
        payload: std::ops::Range<usize>,
    },
    Partial,
}

fn escape(s: &[u8]) -> Escape {
    if s[0] != 0x1b {
        return Escape::None;
    }
    let Some(&kind) = s.get(1) else {
        return Escape::Partial;
    };
    let finish = |end: Option<(usize, usize)>, limit: usize| match end {
        Some((len, text_end)) => Escape::Complete {
            len,
            payload: 2..text_end,
        },
        None if s.len() < limit => Escape::Partial,
        None => Escape::Complete {
            len: 2,
            payload: 0..0,
        },
    };
    match kind {
        b'[' => match s[2..].iter().position(|b| (b'@'..=b'~').contains(b)) {
            Some(n) => Escape::Complete {
                len: n + 3,
                payload: 0..0,
            },
            None => finish(None, CSI_LIMIT),
        },
        b']' | b'P' | b'X' | b'^' | b'_' => finish(
            (2..s.len()).find_map(|i| match s[i] {
                0x07 => Some((i + 1, i)),
                0x1b if s.get(i + 1) == Some(&b'\\') => Some((i + 2, i)),
                _ => None,
            }),
            OSC_LIMIT,
        ),
        _ => Escape::Complete {
            len: 2,
            payload: 0..0,
        },
    }
}

/// Matches of `secrets` in `view` (leftmost-longest) as ranges of `input`, and where a
/// possible secret prefix begins at the end of the view.
fn scan(
    view: &[u8],
    origin: &[usize],
    secrets: &[&[u8]],
    hold: bool,
) -> (Vec<(usize, usize)>, Option<usize>) {
    let mut matches = Vec::new();
    let mut v = 0;
    while v < view.len() {
        // A shorter value must not match while the view may still grow into a longer one.
        let rest = &view[v..];
        if hold
            && secrets
                .iter()
                .any(|s| s.len() > rest.len() && s.starts_with(rest))
        {
            break;
        }
        let longest = secrets
            .iter()
            .filter(|s| view[v..].starts_with(s))
            .map(|s| s.len())
            .max();
        match longest {
            Some(n) => {
                matches.push((origin[v], origin[v + n - 1] + 1));
                v += n;
            }
            None => v += 1,
        }
    }
    let matched_to = matches.last().map_or(0, |&(_, end)| end);
    let first_unmatched = origin.partition_point(|&o| o < matched_to);
    let longest = secrets.iter().map(|s| s.len()).max().unwrap_or(0);
    let tail = (first_unmatched.max(view.len().saturating_sub(longest.saturating_sub(1)))
        ..view.len())
        .find(|&t| {
            secrets
                .iter()
                .any(|s| s.len() > view.len() - t && s.starts_with(&view[t..]))
        });
    (matches, tail.map(|t| origin[t]))
}

/// Redacts every secret in `input`, returning the safe prefix and where the tail begins that
/// may still start a secret (or an unfinished escape) and must be rescanned with the next
/// chunk. Without `hold` the whole input is returned, for streams that are not plain text.
///
/// Two views are matched: the raw bytes, and the text a person reads (escapes and control
/// bytes removed, string-escape payloads kept), so neither an escape inside a value nor a
/// stray escape before it hides the value.
fn redact(input: &[u8], secrets: &[&[u8]], hold: bool) -> (Vec<u8>, usize) {
    let mut view = Vec::with_capacity(input.len());
    let mut origin = Vec::with_capacity(input.len());
    let mut open_escape = None;
    let mut i = 0;
    while i < input.len() {
        let mut step = 1;
        match escape(&input[i..]) {
            Escape::Complete { len, payload } => {
                view.extend_from_slice(&input[i + payload.start..i + payload.end]);
                origin.extend(i + payload.start..i + payload.end);
                step = len;
            }
            Escape::Partial => {
                open_escape = Some(i);
                break;
            }
            Escape::None => {
                let b = input[i];
                if b >= 0x20 && b != 0x7f || matches!(b, b'\t' | b'\n' | b'\r') {
                    view.push(b);
                    origin.push(i);
                }
            }
        }
        i += step;
    }
    let raw_origin: Vec<usize> = (0..input.len()).collect();
    let (mut ranges, tail) = scan(&view, &origin, secrets, hold);
    let (raw_ranges, raw_tail) = scan(input, &raw_origin, secrets, hold);
    ranges.extend(raw_ranges);
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    let mut held_from = if hold {
        [tail, raw_tail, open_escape]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(input.len())
    } else {
        input.len()
    };
    // A match that reaches into the held tail is rescanned with the next chunk.
    for &(start, end) in &merged {
        if end > held_from {
            held_from = held_from.min(start);
        }
    }
    let mut out = Vec::with_capacity(held_from);
    let mut at = 0;
    for (start, end) in merged.into_iter().filter(|&(_, end)| end <= held_from) {
        out.extend_from_slice(&input[at..start]);
        out.extend_from_slice(REDACTED);
        at = end;
    }
    out.extend_from_slice(&input[at..held_from]);
    (out, held_from)
}

impl Protection {
    fn secret_bytes(&self) -> Vec<Zeroizing<String>> {
        self.values
            .iter()
            .chain(&self.retired_values)
            .chain(&self.stored_values)
            .flat_map(|value| secret_variants(value))
            .collect()
    }
}

impl RoomSecretObservations {
    /// Filters one provider output chunk in place. Safe text continues to flow; a possible
    /// secret prefix at the end waits for the next chunk or `flush_streams`.
    pub(in crate::runtime::state) fn protect_stream(&self, room: &str, output: &mut Output) {
        let room = self.room_key(room);
        // The owner lookup takes the session lock, so it never runs under the value lock.
        let external = self.external(room);
        let variants = match self.blocked(room, false) {
            Ok(false) => self.rooms.lock().ok().map(|r| {
                let mut secrets = r[room].secret_bytes();
                secrets.extend(external.iter().flat_map(|v| secret_variants(v)));
                secrets
            }),
            _ => None,
        };
        let Some(variants) = variants else {
            output.bytes = WITHHELD.to_vec();
            return;
        };
        if variants.is_empty() {
            return;
        }
        let key = (
            room.to_string(),
            output.provider_run_id.clone(),
            format!(
                "{:?}|{:?}|{:?}",
                output.kind, output.agent_id, output.merge_key
            ),
        );
        let Ok(mut pending) = self.pending_streams.lock() else {
            output.bytes = WITHHELD.to_vec();
            return;
        };
        let mut input = pending
            .remove(&key)
            .map_or_else(Zeroizing::default, |(_, held)| held);
        input.extend_from_slice(&output.bytes);
        let secrets: Vec<&[u8]> = variants.iter().map(|v| v.as_bytes()).collect();
        // Assistant text, reasoning and native terminal bytes are append-only, so a possible
        // value prefix waits for the next chunk. Tool snapshots replace the earlier text and
        // errors, status and echoes are whole messages, so they are redacted in place.
        let hold = matches!(
            output.kind,
            TerminalOutputKind::ProviderOutput
                | TerminalOutputKind::ProviderReasoning
                | TerminalOutputKind::ProviderTerminal
        );
        let (safe, held_from) = redact(&input, &secrets, hold);
        output.bytes = safe;
        if held_from < input.len() {
            let template = Output {
                provider_run_id: output.provider_run_id.clone(),
                agent_id: output.agent_id.clone(),
                kind: output.kind.clone(),
                merge_key: output.merge_key.clone(),
                bytes: Vec::new(),
            };
            pending.insert(key, (template, Zeroizing::new(input[held_from..].to_vec())));
        }
    }

    /// Ends the streams of one provider run and returns what they still held.
    pub(in crate::runtime::state) fn flush_streams(&self, room: &str, run: &str) -> Vec<Output> {
        let room = self.room_key(room);
        let Ok(mut pending) = self.pending_streams.lock() else {
            return Vec::new();
        };
        let keys: Vec<_> = pending
            .keys()
            .filter(|(r, p, _)| r == room && p == run)
            .cloned()
            .collect();
        keys.into_iter()
            .filter_map(|key| pending.remove(&key))
            .map(|(mut output, held)| {
                // An unfinished prefix of a value is never shown beyond a trivial tail.
                output.bytes = if held.len() <= SHORT_TAIL {
                    held.to_vec()
                } else {
                    REDACTED.to_vec()
                };
                output
            })
            .collect()
    }
}
