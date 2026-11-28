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
            merge_key_suffix: reply.block_key(1),
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
            merge_key_suffix: reply.block_key(1),
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
            merge_key_suffix: reply.block_key(1),
        },
        "the transcript text is paired with the plain row, whatever markup it adds"
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
        Adoption::Streamed { .. }
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
    assert!(drawn_from("};", "```\n};\nfn main() {\n```"));
    assert!(!drawn_from("};", "No code here, only words."));
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
fn a_streamed_code_block_is_paired_with_its_fenced_transcript_block() {
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
            merge_key_suffix: reply.block_key(1),
        }
    );
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
        .all(|delta| delta.merge_key_suffix == reply.block_key(2)));
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

/// A reply as the terminal draws it and as the transcript holds it.
struct Reply {
    markdown: String,
    rows: Vec<String>,
}

fn columns(text: &str) -> usize {
    text.chars()
        .map(|c| if c.len_utf8() >= 3 { 2 } else { 1 })
        .sum()
}

/// Claude's wrapping: between words, and inside a token longer than the row.
fn wrapped(text: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut row = String::new();
    for word in text.split(' ') {
        let mut word = word.to_string();
        while columns(&word) > width {
            let mut cut = String::new();
            while columns(&cut) + 2 <= width {
                cut.push(word.remove(0));
            }
            if !row.is_empty() {
                rows.push(std::mem::take(&mut row));
            }
            rows.push(cut);
        }
        if !row.is_empty() && columns(&row) + 1 + columns(&word) > width {
            rows.push(std::mem::take(&mut row));
        }
        if !row.is_empty() {
            row.push(' ');
        }
        row.push_str(&word);
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

fn generated_reply(rng: &mut rand::rngs::StdRng) -> Reply {
    use rand::Rng;
    const WORDS: [&str; 10] = [
        "alpha",
        "beta-gamma",
        "naïve",
        "日本語",
        "emoji🙂",
        "x_y",
        "v1.2",
        "a.b",
        "(paren)",
        "tool",
    ];
    const CODE_SPANS: [&str; 5] = ["ls *", "a_b_", "$HOME/*", "x*y", "cat *.log"];
    const CODE_LINES: [&str; 10] = [
        "fn main() {",
        "    run();",
        "}",
        "ls *",
        ")",
        "]",
        "};",
        "x = a * b;",
        "# note",
        "_private = 1",
    ];
    // Languages that are also words in the code, and long identifiers that wrap
    // with only symbols left on the last row.
    const FENCE_LANGUAGES: [&str; 8] = ["", "", "sh", "rust", "text", "c", "json", "bash"];
    const CODE_WORDS: [&str; 6] = [
        "$ sh",
        "c = 1;",
        "let rust = 1;",
        "text",
        "$ bash -c x",
        "{}",
    ];
    const IDENTIFIER_ENDS: [&str; 6] = ["();", ")", "];", "},", "?;", ""];
    const ENDINGS: [&str; 6] = ["", "", ".", ",", ":", "!"];
    let mut markdown = String::new();
    let mut rows: Vec<String> = Vec::new();
    for block in 0..rng.gen_range(1..=4) {
        if block > 0 {
            markdown.push_str("\n\n");
            rows.push(String::new());
        }
        match rng.gen_range(0..4) {
            0 => {
                let mut items = Vec::new();
                for _ in 0..rng.gen_range(2..=8) {
                    let word = WORDS[rng.gen_range(0..WORDS.len())];
                    let span = CODE_SPANS[rng.gen_range(0..CODE_SPANS.len())];
                    let (md, drawn) = match rng.gen_range(0..8) {
                        0 => (format!("**{word}**"), word.to_string()),
                        1 => (format!("*{word}*"), word.to_string()),
                        2 => (format!("_{word}_"), word.to_string()),
                        3 => (format!("`{span}`"), span.to_string()),
                        4 => (
                            format!("[{word}](https://e.com/p)"),
                            format!("{word} (https://e.com/p)"),
                        ),
                        5 => {
                            let token =
                                format!("https://e.com/{}", "ab-".repeat(rng.gen_range(10..70)));
                            (token.clone(), token)
                        }
                        _ => (word.to_string(), word.to_string()),
                    };
                    let ending = ENDINGS[rng.gen_range(0..ENDINGS.len())];
                    items.push((format!("{md}{ending}"), format!("{drawn}{ending}")));
                }
                markdown.push_str(
                    &items
                        .iter()
                        .map(|i| i.0.as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                );
                let drawn = items
                    .iter()
                    .map(|i| i.1.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                rows.extend(wrapped(&drawn, 116));
            }
            1 => {
                let word = WORDS[rng.gen_range(0..WORDS.len())];
                markdown.push_str(&format!("## {word} tool"));
                rows.push(format!("{word} tool"));
            }
            2 => {
                markdown.push_str("```");
                markdown.push_str(FENCE_LANGUAGES[rng.gen_range(0..FENCE_LANGUAGES.len())]);
                markdown.push('\n');
                let lines = rng.gen_range(1..=5);
                for line in 0..lines {
                    let code = match rng.gen_range(0..4) {
                        0 => CODE_WORDS[rng.gen_range(0..CODE_WORDS.len())].to_string(),
                        1 => format!(
                            "f{}{}",
                            "a".repeat(rng.gen_range(100..140)),
                            IDENTIFIER_ENDS[rng.gen_range(0..IDENTIFIER_ENDS.len())]
                        ),
                        _ => CODE_LINES[rng.gen_range(0..CODE_LINES.len())].to_string(),
                    };
                    if line > 0 {
                        markdown.push('\n');
                    }
                    markdown.push_str(&code);
                    rows.extend(wrapped(&code, 116));
                }
                markdown.push_str("\n```");
            }
            _ => {
                for item in 0..rng.gen_range(1..=4) {
                    let word = WORDS[rng.gen_range(0..WORDS.len())];
                    if item > 0 {
                        markdown.push('\n');
                    }
                    markdown.push_str(&format!("- **{word}** tool"));
                    rows.push(format!("- {word} tool"));
                }
            }
        }
    }
    Reply { markdown, rows }
}

/// Streams a generated reply the way Claude's terminal draws it.
fn streamed_reply(live: &mut LiveReply, reply: &Reply) -> String {
    let mut screen = vec![format!("● {}", reply.rows[0])];
    screen.extend(reply.rows[1..].iter().map(|row| {
        if row.is_empty() {
            String::new()
        } else {
            format!("  {row}")
        }
    }));
    screen.extend([String::new(), "✻ Working (3s)".to_string()]);
    let rows: Vec<&str> = screen.iter().map(String::as_str).collect();
    read_twice(live, "prompt-1", &draw(&rows))
        .iter()
        .map(|delta| delta.text.as_str())
        .collect()
}

/// The live row is replaced by the transcript text, so the one thing left to
/// get right is that the transcript block finds the row it replaces. A block
/// that does not would be shown twice: plain, then as markdown.
#[test]
fn every_streamed_reply_is_paired_with_its_transcript_block() {
    use rand::SeedableRng;
    let mut rng = rand::rngs::StdRng::seed_from_u64(1425);
    let mut checked = 0;
    for case in 0..600 {
        let reply = generated_reply(&mut rng);
        if reply.rows.len() > 30 || reply.rows.iter().all(String::is_empty) {
            continue;
        }
        let mut live = LiveReply::new(40, 120);
        let streamed = streamed_reply(&mut live, &reply);
        if streamed.is_empty() {
            continue;
        }
        checked += 1;
        assert_eq!(
            live.adopt(&reply.markdown),
            Adoption::Streamed {
                merge_key_suffix: live.block_key(1),
            },
            "case {case}: the transcript block missed its row\nmarkdown: {:?}\nstreamed: {streamed:?}",
            reply.markdown
        );
    }
    assert!(checked > 100, "only {checked} replies were checked");
}

#[test]
fn a_transcript_block_of_another_reply_is_not_paired_with_a_streamed_row() {
    use rand::SeedableRng;
    let mut rng = rand::rngs::StdRng::seed_from_u64(1426);
    let mut checked = 0;
    for case in 0..300 {
        let (first, second) = (generated_reply(&mut rng), generated_reply(&mut rng));
        if first.rows.len() > 30 || first.rows.iter().all(String::is_empty) {
            continue;
        }
        let mut live = LiveReply::new(40, 120);
        let streamed = streamed_reply(&mut live, &first);
        if streamed.is_empty() || drawn_from(&streamed, &second.markdown) {
            continue;
        }
        checked += 1;
        assert_eq!(
            live.adopt(&second.markdown),
            Adoption::NotStreamed,
            "case {case}: {:?} took the row of {streamed:?}",
            second.markdown
        );
    }
    assert!(checked > 100, "only {checked} pairs were checked");
}

// A reader lost to a restart or to the registry's eviction forgets the blocks it handed over. What it drew is still on
// the screen when Claude redraws it, and the reader that takes over must not stream it again as a new row.
const FIRST: &[&str] = &[
    "● Result",
    "  First body one.",
    "  First body two.",
    "  First body three.",
    "",
    "✻ Working (1s)",
];
const FIRST_TEXT: &str = "Result\nFirst body one.\nFirst body two.\nFirst body three.";
const SECOND: &[&str] = &[
    "● Next",
    "  Second body one.",
    "  Second body two.",
    "  Second body three.",
    "",
    "✻ Working (2s)",
];

fn with_state(file: &std::path::Path) -> LiveReply {
    let mut reply = LiveReply::new(40, 120);
    reply.context_file = Some(file.display().to_string());
    reply
}

#[test]
fn a_completed_block_drawn_again_after_the_reader_was_lost_is_not_streamed_again() {
    let dir = crate::test_support::TestTempDir::new("live-reader-lost-completed");
    let context = dir.path().join("context.json");
    let mut reply = with_state(&context);
    assert!(!read_twice(&mut reply, "prompt-1", &draw(FIRST)).is_empty());
    assert!(matches!(reply.adopt(FIRST_TEXT), Adoption::Streamed { .. }));
    // The reader is lost; the one that takes over sees the screen drawn again.
    let mut lost = with_state(&context);
    let again = read_twice(&mut lost, "prompt-1", &draw(FIRST));
    assert!(
        again.is_empty(),
        "the completed block streamed again: {again:?}"
    );
    // A block it has not seen still streams, in a row of its own.
    let later = read_twice(&mut lost, "prompt-1", &draw(SECOND));
    assert!(!later.is_empty());
}

#[test]
fn a_block_handed_over_but_not_yet_adopted_is_not_streamed_again_after_the_reader_was_lost() {
    let dir = crate::test_support::TestTempDir::new("live-reader-lost-handed-over");
    let context = dir.path().join("context.json");
    let mut reply = with_state(&context);
    assert!(!read_twice(&mut reply, "prompt-1", &draw(FIRST)).is_empty());
    // The screen moves on to another block, which hands the first one over.
    assert!(!read_twice(&mut reply, "prompt-1", &draw(SECOND)).is_empty());
    let mut lost = with_state(&context);
    let again = read_twice(&mut lost, "prompt-1", &draw(FIRST));
    assert!(
        again.is_empty(),
        "the handed-over block streamed again: {again:?}"
    );
}

#[test]
fn the_answer_left_on_the_screen_is_not_streamed_under_the_next_prompt_by_a_reader_that_took_over()
{
    let dir = crate::test_support::TestTempDir::new("live-reader-lost-previous");
    let context = dir.path().join("context.json");
    let mut reply = with_state(&context);
    assert!(!read_twice(&mut reply, "prompt-1", &draw(FIRST)).is_empty());
    assert!(matches!(reply.adopt(FIRST_TEXT), Adoption::Streamed { .. }));
    // The reader is lost as the next prompt starts, while the screen still shows the previous answer.
    let mut lost = with_state(&context);
    let deltas = read_twice(&mut lost, "prompt-2", &draw(FIRST));
    assert!(
        deltas.is_empty(),
        "the previous answer streamed under the next prompt: {deltas:?}"
    );
}

#[test]
fn a_block_that_grew_while_the_reader_was_lost_is_not_streamed_again_once_completed() {
    let dir = crate::test_support::TestTempDir::new("live-reader-grew");
    let context = dir.path().join("context.json");
    let mut reply = with_state(&context);
    let streamed = read_twice(&mut reply, "prompt-1", &draw(FIRST));
    let key = streamed
        .first()
        .expect("the block streams")
        .merge_key_suffix
        .clone();
    // The reader is lost. Claude writes the answer further, and its transcript text completes the block.
    let completed = "Result\nFirst body one.\nFirst body two.\nFirst body three.\nFirst body four.\nFirst body five.";
    super::super::reader_state::mark_done(&context.display().to_string(), &key, Some(completed));
    let grown = draw(&[
        "● Result",
        "  First body one.",
        "  First body two.",
        "  First body three.",
        "  First body four.",
        "  First body five.",
        "",
        "✻ Working (3s)",
    ]);
    let mut lost = with_state(&context);
    let again = read_twice(&mut lost, "prompt-1", &grown);
    assert!(
        again.is_empty(),
        "the completed block, grown, streamed again: {again:?}"
    );
}

#[test]
fn completed_redraw_matching_preserves_literal_code_symbols_after_markup() {
    for markdown in [
        "Result\n\nx = 1;\ny = 2;\n\nDone.",
        "# Result\n\n```rust\nx = 1;\ny = 2;\n```\n\n**Done.**",
    ] {
        assert!(rendered_prefix("Result\nx = 1;\ny = 2;\nDone.", markdown));
        assert!(!rendered_prefix(
            "Result\nx += 1;\ny -= 2;\nDone.",
            markdown
        ));
    }
    for code in ["x *= 1;", "x **= 1;", "x -= 1;", "x_1 = 1;", "x[1] = 1;"] {
        assert!(rendered_prefix(
            &format!("Result\n{code}"),
            &format!("Result\n\n`{code}`")
        ));
        assert!(!rendered_prefix(
            "Result\nx = 1;",
            &format!("Result\n\n`{code}`")
        ));
    }
}

#[test]
fn completed_redraw_matching_accepts_rendered_lists_with_and_without_markers() {
    let numbered = "## Plan\n\n1. **Read** the file.\n2. Then `write()` it.";
    assert!(rendered_prefix(
        "Plan\nRead the file.\nThen write() it.",
        numbered
    ));
    assert!(rendered_prefix(
        "Plan\n1. Read the file.\n2. Then write() it.",
        numbered
    ));
    for marker in ["•", "-", "*"] {
        assert!(rendered_prefix(
            &format!("Plan\n{marker} Read the file."),
            "# Plan\n\n- **Read** the file."
        ));
    }
}
