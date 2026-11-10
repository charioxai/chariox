//! MP-08 / MP-09 / MP-10 / MP-11: the reply of a Claude headless turn, read from
//! its terminal while Claude writes it.
//!
//! Claude's transcript carries a text block only once the whole block is done,
//! so a long answer would reach clients at the end. Its terminal shows the text
//! as it comes. Every finished row of the reply is streamed as the terminal
//! draws it: emphasis, code and headings arrive plain. When the transcript's
//! block lands it takes the streamed row's place (see `adopt`).
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, OnceLock};

/// The size of the terminal a Claude run is launched in (`PtyManager::spawn_for_run`).
const SCREEN_ROWS: u16 = 40;
const SCREEN_COLS: u16 = 120;
const BULLET: &str = "● ";
/// A reply row starts two columns in, under the bullet.
const INDENT: &str = "  ";
const STATUS_GLYPHS: [char; 8] = ['✻', '✢', '✶', '✳', '✽', '·', '*', '✱'];
/// Rows of context that tell where the last row taken sits once the reply scrolled.
const ALIGN_ROWS: usize = 4;
const RUNS: usize = 16;
/// Blocks remembered after they were handed over, or streamed and not adopted yet.
const KEPT_BLOCKS: usize = 16;

/// Text to append to the live row of a reply block.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct LiveDelta {
    pub(super) merge_key_suffix: String,
    pub(super) text: String,
    /// Everything streamed for the block so far, this delta included.
    pub(super) text_so_far: String,
}

/// What the transcript's finished text block becomes, given what was streamed.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Adoption {
    /// Nothing was streamed for it: it goes out as usual.
    NotStreamed,
    /// It takes the place of the streamed row. `rest` is what a client that
    /// appended the streamed text still misses; none when the block was drawn
    /// differently from its markdown (the stored row is replaced either way).
    Streamed {
        merge_key_suffix: String,
        rest: Option<String>,
    },
}

static REPLIES: OnceLock<Mutex<HashMap<String, LiveReply>>> = OnceLock::new();

fn replies() -> std::sync::MutexGuard<'static, HashMap<String, LiveReply>> {
    REPLIES
        .get_or_init(Default::default)
        .lock()
        .expect("live reply registry poisoned")
}

/// Feeds the run's terminal output. With `prompt_id`, the reply being written
/// for that prompt is read and what is new comes back.
pub(super) fn read_terminal(
    provider_run_id: &str,
    prompt_id: Option<&str>,
    bytes: &[u8],
) -> Vec<LiveDelta> {
    let mut replies = replies();
    if !replies.contains_key(provider_run_id) && replies.len() >= RUNS {
        if let Some(oldest) = replies
            .iter()
            .min_by_key(|(_, reply)| reply.touched)
            .map(|(id, _)| id.clone())
        {
            replies.remove(&oldest);
        }
    }
    replies
        .entry(provider_run_id.to_string())
        .or_insert_with(|| LiveReply::new(SCREEN_ROWS, SCREEN_COLS))
        .read(prompt_id, bytes)
}

/// The transcript's finished text block, matched with the block streamed for it.
pub(super) fn adopt(provider_run_id: &str, text: &str) -> Adoption {
    replies()
        .get_mut(provider_run_id)
        .map_or(Adoption::NotStreamed, |reply| reply.adopt(text))
}

struct LiveReply {
    screen: vt100::Parser,
    cols: usize,
    touched: std::time::Instant,
    prompt_id: Option<String>,
    block: Option<Block>,
    /// Streamed blocks whose finished text has not arrived yet, oldest first.
    streamed: VecDeque<Streamed>,
    /// First rows of the blocks handed over: the terminal may draw them again.
    finished: VecDeque<String>,
    blocks_started: usize,
    /// Rows the previous read showed: a row counts once two reads in a row show it.
    previous_rows: HashSet<String>,
}

struct Block {
    merge_key_suffix: String,
    /// Every row taken so far, blank ones too.
    rows: Vec<String>,
    /// Columns the last row with text used.
    last_width: usize,
    text: String,
}

struct Streamed {
    merge_key_suffix: String,
    text: String,
}

#[derive(Clone)]
struct Row {
    text: String,
    /// Columns used, the indent included.
    width: usize,
}

impl LiveReply {
    fn new(rows: u16, cols: u16) -> Self {
        Self {
            screen: vt100::Parser::new(rows, cols, 0),
            cols: usize::from(cols),
            touched: std::time::Instant::now(),
            prompt_id: None,
            block: None,
            streamed: VecDeque::new(),
            finished: VecDeque::new(),
            blocks_started: 0,
            previous_rows: HashSet::new(),
        }
    }

    fn read(&mut self, prompt_id: Option<&str>, bytes: &[u8]) -> Vec<LiveDelta> {
        self.touched = std::time::Instant::now();
        self.screen.process(bytes);
        if self.prompt_id.as_deref() != prompt_id {
            self.block = None;
            self.streamed.clear();
            self.finished.clear();
            self.previous_rows.clear();
            self.prompt_id = prompt_id.map(str::to_string);
        }
        if prompt_id.is_none() {
            return Vec::new();
        }
        self.extract().into_iter().collect()
    }

    fn extract(&mut self) -> Option<LiveDelta> {
        let (bullet_visible, rows) =
            reply_rows(&screen_rows(self.screen.screen()), self.block.is_some());
        let confirmed = std::mem::replace(
            &mut self.previous_rows,
            rows.iter().map(|row| row.text.clone()).collect(),
        );
        let first = rows.first()?;
        // The last row may still be written; every row before it is done.
        let done = &rows[..rows.len() - 1];
        if bullet_visible {
            let block_head = self.block.as_ref().and_then(|block| block.rows.first());
            if block_head.is_some_and(|head| *head != first.text) {
                self.hand_block_over();
            }
            if self.block.is_none() {
                if done.is_empty()
                    || self.finished.contains(&first.text)
                    || !confirmed.contains(&first.text)
                {
                    return None;
                }
                self.blocks_started += 1;
                let prompt_id = self.prompt_id.as_deref().unwrap_or_default();
                self.block = Some(Block {
                    merge_key_suffix: format!(
                        "assistant:live-{prompt_id}-{}:0",
                        self.blocks_started
                    ),
                    rows: Vec::new(),
                    last_width: 0,
                    text: String::new(),
                });
            }
        }
        // A new block starts at its bullet. Once rows were taken, the screen
        // may have scrolled or been drawn again: find the last rows taken.
        let taken = &self.block.as_ref()?.rows;
        let start = if taken.is_empty() {
            if !bullet_visible {
                return None;
            }
            0
        } else {
            first_new_row(taken, done)?
        };
        let cols = self.cols;
        let block = self.block.as_mut()?;
        let mut delta = String::new();
        for row in done.iter().skip(start) {
            if row.text.is_empty() {
                block.rows.push(String::new());
                continue;
            }
            if !confirmed.contains(&row.text) {
                break;
            }
            let joint = match block.rows.last() {
                None => "",
                Some(previous) if previous.is_empty() => "\n\n",
                Some(_) if wraps_into(block.last_width, &row.text, cols) => " ",
                Some(_) => "\n",
            };
            delta.push_str(joint);
            delta.push_str(if joint == " " {
                row.text.trim_start()
            } else {
                &row.text
            });
            block.rows.push(row.text.clone());
            block.last_width = row.width;
        }
        if delta.is_empty() {
            return None;
        }
        // Blank rows between two paragraphs come before the paragraph that follows.
        if block.text.is_empty() {
            delta = delta.trim_start().to_string();
        }
        block.text.push_str(&delta);
        Some(LiveDelta {
            merge_key_suffix: block.merge_key_suffix.clone(),
            text: delta,
            text_so_far: block.text.clone(),
        })
    }

    fn hand_block_over(&mut self) {
        let Some(block) = self.block.take() else {
            return;
        };
        if let Some(first_row) = block.rows.first() {
            self.finished.push_back(first_row.clone());
            if self.finished.len() > KEPT_BLOCKS {
                self.finished.pop_front();
            }
        }
        if !block.text.is_empty() {
            self.streamed.push_back(Streamed {
                merge_key_suffix: block.merge_key_suffix,
                text: block.text,
            });
            if self.streamed.len() > KEPT_BLOCKS {
                self.streamed.pop_front();
            }
        }
    }

    fn adopt(&mut self, text: &str) -> Adoption {
        // The block being streamed is done once its text is in the transcript.
        if self
            .block
            .as_ref()
            .is_some_and(|block| drawn_from(&block.text, text))
        {
            self.hand_block_over();
        }
        let Some(index) = self
            .streamed
            .iter()
            .position(|streamed| drawn_from(&streamed.text, text))
        else {
            return Adoption::NotStreamed;
        };
        let streamed = self.streamed.remove(index).expect("index found above");
        Adoption::Streamed {
            rest: text
                .strip_prefix(streamed.text.as_str())
                .map(str::to_string),
            merge_key_suffix: streamed.merge_key_suffix,
        }
    }
}

/// Whether `drawn` is the start of `markdown` as the terminal shows it: the
/// letters and digits of what was drawn come in order, close together, at the
/// start of the markdown, which the terminal shows without its markup.
fn drawn_from(drawn: &str, markdown: &str) -> bool {
    /// Letters of markup (a code fence's language) the terminal may leave out.
    const MARKUP_LETTERS: usize = 16;
    let letters = |text: &str| {
        text.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<Vec<_>>()
    };
    let (drawn, source) = (letters(drawn), letters(markdown));
    if drawn.is_empty() {
        return false;
    }
    let mut position = 0;
    for letter in &drawn {
        let Some(found) = source[position..].iter().position(|c| c == letter) else {
            return false;
        };
        position += found + 1;
    }
    position <= drawn.len() + drawn.len() / 2 + MARKUP_LETTERS
}

fn screen_rows(screen: &vt100::Screen) -> Vec<Row> {
    let (rows, cols) = screen.size();
    (0..rows)
        .map(|row| {
            let mut text = String::new();
            let mut width = 0;
            for col in 0..cols {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                if !cell.has_contents() {
                    text.push(' ');
                    continue;
                }
                text.push_str(cell.contents());
                width = usize::from(col) + if cell.is_wide() { 2 } else { 1 };
            }
            Row {
                text: text.trim_end().to_string(),
                width,
            }
        })
        .collect()
}

fn is_tool_row(text: &str) -> bool {
    let Some(name) = text.strip_prefix(BULLET) else {
        return false;
    };
    let word = name.split('(').next().unwrap_or_default();
    name.contains('(')
        && word.chars().next().is_some_and(char::is_uppercase)
        && word.chars().all(char::is_alphanumeric)
}

fn ends_reply(text: &str) -> bool {
    text.starts_with("──")
        || text.starts_with('❯')
        || text.starts_with('⏵')
        || text.starts_with("  ⎿")
        || text
            .chars()
            .next()
            .is_some_and(|glyph| STATUS_GLYPHS.contains(&glyph))
            && text.chars().nth(1) == Some(' ')
}

/// The rows of the reply last drawn, the bullet and the indent taken off, and
/// whether its bullet row is on the screen (it is not once the reply has
/// scrolled past the top). The last bullet may be a tool's, which Claude draws
/// as `Tool(...)` or as a summary with its progress under it (`⎿`).
fn reply_rows(rows: &[Row], scrolled_reply: bool) -> (bool, Vec<Row>) {
    let bullet = rows
        .iter()
        .rposition(|row| row.text.starts_with(BULLET))
        .filter(|index| !is_tool_row(&rows[*index].text));
    let first = match bullet {
        Some(index) => index,
        None if scrolled_reply => 0,
        None => return (false, Vec::new()),
    };
    let mut reply = Vec::new();
    for (offset, row) in rows[first..].iter().enumerate() {
        let text = if offset == 0 && bullet.is_some() {
            row.text[BULLET.len()..].to_string()
        } else if row.text.starts_with("  ⎿") && bullet.is_some() {
            return (false, Vec::new());
        } else if ends_reply(&row.text) || (!row.text.is_empty() && !row.text.starts_with(INDENT)) {
            break;
        } else {
            row.text
                .strip_prefix(INDENT)
                .unwrap_or(&row.text)
                .to_string()
        };
        reply.push(Row {
            text,
            ..row.clone()
        });
    }
    while reply.last().is_some_and(|row| row.text.is_empty()) {
        reply.pop();
    }
    (bullet.is_some(), reply)
}

/// Where the rows not taken yet start in `done`: after the last rows taken,
/// and after the blank rows already taken behind them.
fn first_new_row(taken: &[String], done: &[Row]) -> Option<usize> {
    let blank_tail = taken.iter().rev().take_while(|row| row.is_empty()).count();
    let taken = &taken[..taken.len() - blank_tail];
    let end = (1..=ALIGN_ROWS.min(taken.len())).rev().find_map(|size| {
        let context = &taken[taken.len() - size..];
        (size..=done.len()).rev().find(|end| {
            done[end - size..*end]
                .iter()
                .map(|row| row.text.as_str())
                .eq(context.iter().map(String::as_str))
        })
    })?;
    let blanks_seen = done[end..]
        .iter()
        .take_while(|row| row.text.is_empty())
        .count();
    Some(end + blanks_seen.min(blank_tail))
}

/// Claude wraps a paragraph between words, before a word would reach the
/// screen's last column: a row that stopped shorter ended its line.
fn wraps_into(previous_width: usize, next_row: &str, cols: usize) -> bool {
    let next_word = next_row.trim_start().split(' ').next().unwrap_or_default();
    previous_width + 1 + next_word.chars().count() >= cols
}

#[cfg(test)]
#[path = "live_reply_tests.rs"]
mod tests;
