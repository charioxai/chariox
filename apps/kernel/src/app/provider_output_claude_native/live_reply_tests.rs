use super::*;
use base64::Engine;

// Real Claude Code 2.1.296 sessions, recorded from the terminal: [seconds, base64 bytes] per read.
const SCROLLED: &str = include_str!("tests/fixtures/claude-2.1.296-reply-24x120.pty.jsonl");
const SCROLLED_REPLY: &str = include_str!("tests/fixtures/claude-2.1.296-reply-24x120.reply.txt");
const PLAIN: &str = include_str!("tests/fixtures/claude-2.1.296-reply-40x120.pty.jsonl");
const PLAIN_REPLY: &str = include_str!("tests/fixtures/claude-2.1.296-reply-40x120.reply.txt");
const EMPHASIS: &str = include_str!("tests/fixtures/claude-2.1.296-reply-with-emphasis.pty.jsonl");
const EMPHASIS_REPLY: &str =
    include_str!("tests/fixtures/claude-2.1.296-reply-with-emphasis.reply.txt");
const HEADING: &str = include_str!("tests/fixtures/claude-2.1.296-reply-with-heading.pty.jsonl");
const HEADING_REPLY: &str =
    include_str!("tests/fixtures/claude-2.1.296-reply-with-heading.reply.txt");
/// A 10,561-character essay: the reply scrolls, and Claude draws the screen again mid-reply.
const ESSAY: &[u8] = include_bytes!("tests/fixtures/claude-2.1.296-essay-40x120.pty.jsonl.gz");
const ESSAY_REPLY: &str = include_str!("tests/fixtures/claude-2.1.296-essay-40x120.reply.txt");

fn gunzip(bytes: &[u8]) -> String {
    use std::io::Read;
    let mut text = String::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_string(&mut text)
        .unwrap();
    text
}

/// The terminal reads of a recording, with the second each arrived at.
fn terminal_output(recording: &str) -> Vec<(f64, Vec<u8>)> {
    recording
        .lines()
        .map(|line| {
            let read: serde_json::Value = serde_json::from_str(line).unwrap();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(read[1].as_str().unwrap())
                .unwrap();
            (read[0].as_f64().unwrap(), bytes)
        })
        .collect()
}

struct Streamed {
    deltas: Vec<LiveDelta>,
    /// When the terminal first showed a row of the reply.
    shown_at: Option<f64>,
    /// When the first text was streamed.
    first_text_at: Option<f64>,
}

impl Streamed {
    fn text(&self) -> String {
        self.deltas
            .iter()
            .map(|delta| delta.text.as_str())
            .collect()
    }
}

/// What the pump does: reads the screen after every read of the terminal.
/// `finished_text` tells when the terminal first showed a row of the reply.
fn stream(reply: &mut LiveReply, reads: &[(f64, Vec<u8>)], finished_text: &str) -> Streamed {
    let mut streamed = Streamed {
        deltas: Vec::new(),
        shown_at: None,
        first_text_at: None,
    };
    for (at, bytes) in reads {
        let deltas = reply.read(Some("prompt-1"), bytes);
        if streamed.shown_at.is_none()
            && reply_rows(&screen_rows(reply.screen.screen()), false)
                .1
                .first()
                .is_some_and(|row| !row.text.is_empty() && drawn_from(&row.text, finished_text))
        {
            streamed.shown_at = Some(*at);
        }
        if streamed.first_text_at.is_none() && !deltas.is_empty() {
            streamed.first_text_at = Some(*at);
        }
        streamed.deltas.extend(deltas);
    }
    streamed
}

fn assert_streams_as_shown(streamed: &Streamed, finished_text: &str, at_least_percent: usize) {
    let text = streamed.text();
    assert!(
        drawn_from(&text, finished_text),
        "what was streamed is not the reply as drawn:\n{text}"
    );
    assert!(
        text.len() * 100 >= finished_text.len() * at_least_percent,
        "only {} of {} bytes streamed",
        text.len(),
        finished_text.len()
    );
    let lag = streamed.first_text_at.unwrap() - streamed.shown_at.unwrap();
    assert!(
        lag <= 1.0,
        "the first text came {lag:.2}s after the terminal showed it"
    );
    let key = &streamed.deltas[0].merge_key_suffix;
    assert!(
        streamed
            .deltas
            .iter()
            .all(|delta| delta.merge_key_suffix == *key),
        "one block is one row"
    );
    assert_eq!(streamed.deltas.last().unwrap().text_so_far, text);
}

#[test]
fn a_reply_taller_than_the_screen_streams_row_by_row_and_the_transcript_continues_it() {
    let mut reply = LiveReply::new(24, 120);
    let streamed = stream(&mut reply, &terminal_output(SCROLLED), SCROLLED_REPLY);
    assert_streams_as_shown(&streamed, SCROLLED_REPLY, 95);
    assert!(
        streamed.deltas.len() >= 10,
        "{} pieces",
        streamed.deltas.len()
    );
    let text = streamed.text();
    assert!(
        SCROLLED_REPLY.starts_with(&text),
        "plain prose streams exactly"
    );
    assert_eq!(
        reply.adopt(SCROLLED_REPLY),
        Adoption::Streamed {
            merge_key_suffix: "assistant:live-prompt-1-1:0".to_string(),
            rest: Some(SCROLLED_REPLY[text.len()..].to_string()),
        }
    );
}

#[test]
fn a_long_essay_streams_exactly_through_scrolls_and_redraws() {
    let mut reply = LiveReply::new(40, 120);
    let streamed = stream(&mut reply, &terminal_output(&gunzip(ESSAY)), ESSAY_REPLY);
    assert_streams_as_shown(&streamed, ESSAY_REPLY, 99);
    assert!(
        streamed.deltas.len() >= 50,
        "{} pieces",
        streamed.deltas.len()
    );
    let text = streamed.text();
    assert!(
        ESSAY_REPLY.starts_with(&text),
        "streamed text left the transcript's at byte {}",
        text.bytes()
            .zip(ESSAY_REPLY.bytes())
            .take_while(|(a, b)| a == b)
            .count()
    );
    assert_eq!(
        reply.adopt(ESSAY_REPLY),
        Adoption::Streamed {
            merge_key_suffix: "assistant:live-prompt-1-1:0".to_string(),
            rest: Some(ESSAY_REPLY[text.len()..].to_string()),
        }
    );
}

#[test]
fn a_reply_that_fits_the_screen_streams_the_same_way() {
    let mut reply = LiveReply::new(40, 120);
    let streamed = stream(&mut reply, &terminal_output(PLAIN), PLAIN_REPLY);
    assert_streams_as_shown(&streamed, PLAIN_REPLY, 90);
    assert!(PLAIN_REPLY.starts_with(&streamed.text()));
}

#[test]
fn emphasis_streams_plain_and_the_transcript_text_replaces_it() {
    let mut reply = LiveReply::new(40, 120);
    let streamed = stream(&mut reply, &terminal_output(EMPHASIS), EMPHASIS_REPLY);
    assert_streams_as_shown(&streamed, EMPHASIS_REPLY, 80);
    assert!(
        !EMPHASIS_REPLY.starts_with(&streamed.text()),
        "the row with `echo one` is drawn without its backticks"
    );
    assert_eq!(
        reply.adopt(EMPHASIS_REPLY),
        Adoption::Streamed {
            merge_key_suffix: "assistant:live-prompt-1-1:0".to_string(),
            rest: None,
        }
    );
}

#[test]
fn a_heading_and_a_list_stream_plain_too() {
    let mut reply = LiveReply::new(40, 120);
    let streamed = stream(&mut reply, &terminal_output(HEADING), HEADING_REPLY);
    assert_eq!(
        streamed.text(),
        "Heading Two\n\nA paragraph with bold, italic, code and a link (https://example.com/x) inside.\n\n- first bullet\n- second bullet with bold"
    );
    assert!(matches!(
        reply.adopt(HEADING_REPLY),
        Adoption::Streamed { rest: None, .. }
    ));
}

#[test]
fn a_finished_reply_drawn_again_does_not_stream_twice() {
    let reads = terminal_output(PLAIN);
    let mut reply = LiveReply::new(40, 120);
    stream(&mut reply, &reads, PLAIN_REPLY);
    assert!(matches!(
        reply.adopt(PLAIN_REPLY),
        Adoption::Streamed { .. }
    ));
    let again = stream(&mut reply, &reads[reads.len() - 20..], PLAIN_REPLY);
    assert!(again.deltas.is_empty(), "{:?}", again.text());
    assert_eq!(reply.adopt(PLAIN_REPLY), Adoption::NotStreamed);
}

#[test]
fn another_text_is_not_taken_for_the_streamed_block() {
    let mut reply = LiveReply::new(40, 120);
    stream(&mut reply, &terminal_output(PLAIN), PLAIN_REPLY);
    assert_eq!(
        reply.adopt("Done. The note is above."),
        Adoption::NotStreamed
    );
    assert!(matches!(
        reply.adopt(PLAIN_REPLY),
        Adoption::Streamed { .. }
    ));
}

#[test]
fn a_new_prompt_starts_with_nothing_streamed() {
    let reads = terminal_output(PLAIN);
    let mut reply = LiveReply::new(40, 120);
    assert!(!stream(&mut reply, &reads, PLAIN_REPLY).deltas.is_empty());
    assert!(reply.read(Some("prompt-2"), &[]).is_empty());
    assert_eq!(reply.adopt(PLAIN_REPLY), Adoption::NotStreamed);
}

#[test]
fn nothing_is_read_while_no_prompt_is_active() {
    let reads = terminal_output(PLAIN);
    let mut reply = LiveReply::new(40, 120);
    let deltas = reads
        .iter()
        .flat_map(|(_, bytes)| reply.read(None, bytes))
        .count();
    assert_eq!(deltas, 0);
}

#[test]
fn rows_join_the_way_claude_wrapped_them() {
    // A row that stopped short of the edge ended its line; one that reached it wrapped.
    assert!(wraps_into(118, "of the harbor", 120));
    assert!(!wraps_into(60, "of the harbor", 120));
    assert!(wraps_into(110, "considerably", 120));
    // Claude moves a word that would reach the last column to the next row.
    assert!(wraps_into(116, "its", 120));
    assert!(!wraps_into(115, "its", 120));
}

#[test]
fn drawn_text_is_matched_with_its_markdown() {
    assert!(drawn_from(
        "A paragraph with bold, code and a link (https://example.com/x)",
        "A paragraph with **bold**, `code` and a [link](https://example.com/x) inside."
    ));
    assert!(drawn_from("fn main() {}", "```rust\nfn main() {}\n```"));
    assert!(!drawn_from("Summary", "Some unrelated text that mentions many letters in a row: s u m m a r y and a long tail after it that goes on and on for a while"));
    assert!(!drawn_from("", "anything"));
}
