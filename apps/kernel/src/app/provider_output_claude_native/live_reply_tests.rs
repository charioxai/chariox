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
            rest: SCROLLED_REPLY[text.len()..].to_string(),
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
            rest: ESSAY_REPLY[text.len()..].to_string(),
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
    let Adoption::Streamed {
        merge_key_suffix,
        rest,
    } = reply.adopt(EMPHASIS_REPLY)
    else {
        panic!("the streamed block was not adopted");
    };
    assert_eq!(merge_key_suffix, "assistant:live-prompt-1-1:0");
    assert!(
        !rest.is_empty() && EMPHASIS_REPLY.ends_with(&rest),
        "the tail the terminal had not drawn yet: {rest:?}"
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
        Adoption::Streamed { rest, .. } if rest == "\n\nFinal plain line."
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
fn the_answer_left_on_the_screen_is_not_streamed_again_under_the_next_prompt() {
    let reads = terminal_output(PLAIN);
    let mut reply = LiveReply::new(40, 120);
    stream(&mut reply, &reads, PLAIN_REPLY);
    // The screen still shows the previous answer while the new turn only spins.
    for _ in 0..3 {
        let deltas = reply.read(Some("prompt-2"), b"");
        assert!(deltas.is_empty(), "{deltas:?}");
    }
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

/// The screen of a Claude run, drawn from the top: `rows` as they are written.
fn draw(rows: &[&str]) -> Vec<u8> {
    let mut screen = b"\x1b[2J\x1b[H".to_vec();
    for row in rows {
        screen.extend_from_slice(row.as_bytes());
        screen.extend_from_slice(b"\r\n");
    }
    screen
}

fn read_twice(reply: &mut LiveReply, prompt_id: &str, screen: &[u8]) -> Vec<LiveDelta> {
    let mut deltas = reply.read(Some(prompt_id), screen);
    deltas.extend(reply.read(Some(prompt_id), b""));
    deltas
}

#[test]
fn the_tail_after_a_streamed_code_block_does_not_repeat_its_closing_brace() {
    let mut reply = LiveReply::new(40, 120);
    let deltas = read_twice(
        &mut reply,
        "prompt-1",
        &draw(&[
            "● Here is the loop:",
            "",
            "  fn main() {",
            "      run();",
            "  }",
            "",
            "  It runs once and stops.",
            "",
            "✻ Working (3s)",
        ]),
    );
    let streamed: String = deltas.iter().map(|delta| delta.text.as_str()).collect();
    assert_eq!(streamed, "Here is the loop:\n\nfn main() {\n    run();\n}");
    let markdown =
        "Here is the loop:\n\n```rust\nfn main() {\n    run();\n}\n```\n\nIt runs once and stops.";
    assert_eq!(
        reply.adopt(markdown),
        Adoption::Streamed {
            merge_key_suffix: "assistant:live-prompt-1-1:0".to_string(),
            rest: "\n\nIt runs once and stops.".to_string(),
        }
    );
}

#[test]
fn a_withheld_closing_row_of_a_code_block_is_sent_without_its_fence() {
    let mut reply = LiveReply::new(40, 120);
    read_twice(
        &mut reply,
        "prompt-1",
        &draw(&[
            "● Run it:",
            "",
            "  fn main() {",
            "      run();",
            "  }",
            "",
            "✻ Working (3s)",
        ]),
    );
    let Adoption::Streamed { rest, .. } =
        reply.adopt("Run it:\n\n```rust\nfn main() {\n    run();\n}\n```")
    else {
        panic!("the streamed block was not adopted");
    };
    assert_eq!(rest, "\n}");
}

#[test]
fn a_new_answer_that_opens_with_the_previous_answers_first_line_still_streams() {
    let mut reply = LiveReply::new(40, 120);
    read_twice(
        &mut reply,
        "prompt-1",
        &draw(&[
            "● Summary",
            "  Old body one.",
            "  Old body two.",
            "  Old body three.",
            "",
            "✻ Working (1s)",
        ]),
    );
    // The new turn only spins while the old answer stays on the screen.
    assert!(read_twice(&mut reply, "prompt-2", b"").is_empty());
    let deltas = read_twice(
        &mut reply,
        "prompt-2",
        &draw(&[
            "● Summary",
            "  New body one.",
            "  New body two.",
            "  New body three.",
            "",
            "✻ Working (2s)",
        ]),
    );
    let streamed: String = deltas.iter().map(|delta| delta.text.as_str()).collect();
    assert_eq!(streamed, "Summary\nNew body one.\nNew body two.");
    assert!(deltas
        .iter()
        .all(|delta| delta.merge_key_suffix.contains("prompt-2")));
}

#[test]
fn a_second_block_of_a_turn_may_open_with_the_line_of_a_block_the_transcript_has() {
    let mut reply = LiveReply::new(40, 120);
    let first = read_twice(
        &mut reply,
        "prompt-1",
        &draw(&[
            "● Result",
            "  First body one.",
            "  First body two.",
            "  First body three.",
            "",
            "✻ Working (1s)",
        ]),
    );
    assert_eq!(
        first.last().unwrap().text_so_far,
        "Result\nFirst body one.\nFirst body two."
    );
    // Claude writes the first block to its transcript before it opens the next.
    assert!(matches!(
        reply.adopt("Result\nFirst body one.\nFirst body two.\nFirst body three."),
        Adoption::Streamed { .. }
    ));
    let second = read_twice(
        &mut reply,
        "prompt-1",
        &draw(&[
            "● Result",
            "  Second body one.",
            "  Second body two.",
            "  Second body three.",
            "",
            "✻ Working (2s)",
        ]),
    );
    let streamed: String = second.iter().map(|delta| delta.text.as_str()).collect();
    assert_eq!(streamed, "Result\nSecond body one.\nSecond body two.");
    assert!(second
        .iter()
        .all(|delta| delta.merge_key_suffix == "assistant:live-prompt-1-2:0"));
}

#[test]
fn a_block_drawn_again_is_not_streamed_a_second_time() {
    let mut reply = LiveReply::new(40, 120);
    let screen = draw(&[
        "● Result",
        "  First body one.",
        "  First body two.",
        "  First body three.",
        "",
        "✻ Working (1s)",
    ]);
    read_twice(&mut reply, "prompt-1", &screen);
    // A new block opens, then the first one is drawn again.
    read_twice(
        &mut reply,
        "prompt-1",
        &draw(&[
            "● Other",
            "  Other one.",
            "  Other two.",
            "  Other three.",
            "",
            "✻ Working",
        ]),
    );
    assert!(read_twice(&mut reply, "prompt-1", &screen).is_empty());
}

#[test]
fn a_repeated_line_is_confirmed_as_often_as_it_was_shown() {
    let mut reply = LiveReply::new(40, 120);
    read_twice(
        &mut reply,
        "prompt-1",
        &draw(&["● Start", "  }", "  middle", "  end", "", "✻ Working (1s)"]),
    );
    // A second `}` appears for the first time: one read cannot confirm it.
    let deltas = reply.read(
        Some("prompt-1"),
        &draw(&[
            "● Start",
            "  }",
            "  middle",
            "  }",
            "  more",
            "",
            "✻ Working (2s)",
        ]),
    );
    let streamed: String = deltas.iter().map(|delta| delta.text.as_str()).collect();
    assert!(!streamed.ends_with('}'), "{streamed:?}");
}

/// Streams `rows` as the reply of a prompt and has the transcript block `markdown` adopt it.
fn adopted_tail(rows: &[&str], markdown: &str) -> (String, String) {
    let mut reply = LiveReply::new(40, 120);
    let mut screen = rows.to_vec();
    screen.extend(["", "✻ Working (3s)"]);
    let deltas = read_twice(&mut reply, "prompt-1", &draw(&screen));
    let streamed = deltas.iter().map(|delta| delta.text.as_str()).collect();
    let Adoption::Streamed { rest, .. } = reply.adopt(markdown) else {
        panic!("the streamed block was not adopted");
    };
    (streamed, rest)
}

#[test]
fn the_wildcard_of_a_streamed_command_is_not_sent_again_with_the_tail() {
    let (streamed, rest) = adopted_tail(
        &["● Run this:", "", "  ls *", "", "  Then check the output."],
        "Run this:\n\n```\nls *\n```\n\nThen check the output.",
    );
    assert_eq!(streamed, "Run this:\n\nls *");
    assert_eq!(rest, "\n\nThen check the output.");
}

#[test]
fn an_inline_command_ending_in_a_wildcard_is_not_sent_again() {
    let (streamed, rest) = adopted_tail(
        &["● Try ls *", "", "  Then check."],
        "Try `ls *`\n\nThen check.",
    );
    assert_eq!(streamed, "Try ls *");
    assert_eq!(rest, "\n\nThen check.");
}

#[test]
fn a_literal_underscore_or_backtick_ending_the_streamed_text_is_not_sent_again_nor_its_closers() {
    let (streamed, rest) = adopted_tail(
        &["● Rename it to x_", "", "  Done."],
        "Rename it to `x_`\n\nDone.",
    );
    assert_eq!(
        (streamed.as_str(), rest.as_str()),
        ("Rename it to x_", "\n\nDone.")
    );
    let (streamed, rest) = adopted_tail(
        &["● Use echo `", "", "  Done."],
        "Use `` echo ` ``\n\nDone.",
    );
    assert_eq!(
        (streamed.as_str(), rest.as_str()),
        ("Use echo `", "\n\nDone.")
    );
    let (streamed, rest) = adopted_tail(
        &["● It is bold *", "", "  Done."],
        "It is **bold** *\n\nDone.",
    );
    assert_eq!(
        (streamed.as_str(), rest.as_str()),
        ("It is bold *", "\n\nDone.")
    );
}

#[test]
fn a_code_fence_closing_the_text_leaves_nothing_to_send() {
    let (streamed, rest) = adopted_tail(
        &["● Run this:", "", "  ls *", "  cat *.log"],
        "Run this:\n\n```\nls *\ncat *.log\n```",
    );
    assert_eq!(streamed, "Run this:\n\nls *");
    assert_eq!(rest, "\ncat *.log");
    let (_, rest) = adopted_tail(
        &["● Run this:", "", "  ls *", "  cat *.log", "", "  Done."],
        "Run this:\n\n```\nls *\ncat *.log\n```\n\nDone.",
    );
    assert_eq!(rest, "\n\nDone.");
}
