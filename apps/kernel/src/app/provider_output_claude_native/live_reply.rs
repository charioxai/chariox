//! MP-08 / MP-09 / MP-10 / MP-11: the reply of a Claude headless turn, read from
//! its terminal while Claude writes it.
//!
//! Claude's transcript carries a text block only once the whole block is done,
//! so a long answer would reach clients at the end. Its terminal shows the text
//! as it comes. Every finished row of the reply is streamed as the terminal
//! draws it: emphasis, code and headings arrive plain. When the transcript's
//! block lands it takes the streamed row's place (see `adopt`).
use std::collections::{HashMap, VecDeque};
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

/// A block of the prompt that was being streamed when the reader was lost, as history still holds it. A reader that
/// starts afresh may go on in this row instead of beginning another one.
#[derive(Clone, Debug)]
pub(super) struct Unfinished {
    pub(super) merge_key_suffix: String,
    pub(super) text: String,
}

/// What the transcript's finished text block becomes, given what was streamed.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Adoption {
    /// Nothing was streamed for it: it goes out as usual.
    NotStreamed,
    /// It replaces the row streamed under `merge_key_suffix`: the terminal's
    /// plain rendering of it.
    Streamed { merge_key_suffix: String },
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
    unfinished: Option<Unfinished>,
    context_file: Option<&str>,
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
    let reply = replies
        .entry(provider_run_id.to_string())
        .or_insert_with(|| LiveReply::new(SCREEN_ROWS, SCREEN_COLS));
    reply.context_file = context_file.map(str::to_string);
    if reply.is_fresh() {
        reply.unfinished = unfinished;
    }
    reply.read(prompt_id, bytes)
}

/// A block was completed without the reader (its transcript text arrived for a row the reader no longer knew): what was
/// kept for the run changed, and the reader, if there is one, takes it in again on its next read.
pub(super) fn refresh_taken(provider_run_id: &str) {
    if let Some(reply) = replies().get_mut(provider_run_id) {
        reply.recalled = false;
    }
}

/// Whether the run's reader has streamed nothing yet (it was just created, or the run has none): only such a reader
/// can be the one that replaced a reader lost in the middle of a block.
pub(super) fn is_fresh(provider_run_id: &str) -> bool {
    replies()
        .get(provider_run_id)
        .is_none_or(LiveReply::is_fresh)
}

/// What a kernel restart does to the reader: it is in memory only.
#[cfg(test)]
pub(super) fn forget(provider_run_id: &str) {
    replies().remove(provider_run_id);
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
    /// Rows taken of the blocks handed over: the terminal may draw them again.
    finished: VecDeque<Vec<String>>,
    /// The rows taken of blocks completed while no reader had them in memory, with the transcript text that completed
    /// them: a redraw of such a block shows more rows than the reader had taken when it was lost, so it is the rows taken
    /// exactly, followed by more drawn from that text.
    completed: VecDeque<(Vec<String>, String)>,
    /// Which reader this is. Block identities are persisted (they are merge keys), and a reader lost to a restart or to
    /// the run registry's eviction counts its blocks from 1 again: the reader's own number keeps its keys from ever
    /// being the earlier reader's.
    reader: u64,
    blocks_started: usize,
    /// Rows the previous read showed, with how often: a row counts once two
    /// reads in a row show it that many times.
    previous_rows: HashMap<String, usize>,
    /// The rows of the answer the screen showed when the prompt changed: it
    /// is the previous turn's, so it is not streamed under the new prompt
    /// until the screen shows something other than it.
    left_over: Option<Vec<String>>,
    /// Rows the screen may show beyond `left_over`: one when it was recalled from the blocks a lost reader had taken.
    left_over_slack: usize,
    /// The completing text of the block `left_over` was recalled from, when there is one.
    left_over_text: Option<String>,
    /// The unfinished block of an earlier reader, which the first block this reader streams may go on in.
    unfinished: Option<Unfinished>,
    /// The run's context file: what the reader must not forget when it is lost is kept next to it.
    context_file: Option<String>,
    /// Whether what an earlier reader left next to the context file has been taken in (once, on the first read of a prompt).
    recalled: bool,
}

struct Block {
    merge_key_suffix: String,
    /// Every row taken so far, blank ones too.
    rows: Vec<String>,
    /// Columns the last row with text used.
    last_width: usize,
    text: String,
    /// How much of `text` clients already hold, from the row an earlier reader began: only what is beyond it is sent.
    shown: usize,
    /// How many of `rows` are kept next to the context file (see `reader_state`).
    kept: usize,
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
    /// Printed whitespace, excluding the unoccupied cells at the end of the terminal row.
    trailing: String,
}

/// A number no earlier reader had, in this process or before a restart: the wall clock at the first reader, then one more
/// for each reader after it.
fn next_reader() -> u64 {
    static NEXT: OnceLock<std::sync::atomic::AtomicU64> = OnceLock::new();
    NEXT.get_or_init(|| std::sync::atomic::AtomicU64::new(crate::session::unix_epoch_ms()))
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl LiveReply {
    fn is_fresh(&self) -> bool {
        self.blocks_started == 0 && self.block.is_none() && self.streamed.is_empty()
    }

    /// The merge key of this reader's `block`th block for the prompt it reads. Adoption finds a row left by an earlier
    /// reader by the prefix `live-{prompt}-`, so that stays the start of every key.
    fn block_key(&self, block: usize) -> String {
        let prompt_id = self.prompt_id.as_deref().unwrap_or_default();
        format!("assistant:live-{prompt_id}-{}.{block}:0", self.reader)
    }

    /// What the reader that was lost had taken: the blocks it handed over for this prompt are not streamed again when
    /// Claude draws them again, and the answer of the prompt before, still on the screen, is not taken for a new one.
    fn recall_taken_blocks(&mut self) {
        let (Some(context_file), Some(prompt_id)) =
            (self.context_file.as_deref(), self.prompt_id.clone())
        else {
            return;
        };
        let taken = super::reader_state::recall(context_file);
        for block in taken
            .iter()
            .filter(|block| block.done && block.prompt_id == prompt_id)
        {
            if !self.finished.contains(&block.rows) {
                if self.finished.len() >= KEPT_BLOCKS {
                    self.finished.pop_front();
                }
                self.finished.push_back(block.rows.clone());
            }
            if let Some(text) = &block.completed {
                let kept = (block.rows.clone(), text.clone());
                if !self.completed.contains(&kept) {
                    if self.completed.len() >= KEPT_BLOCKS {
                        self.completed.pop_front();
                    }
                    self.completed.push_back(kept);
                }
            }
        }
        if self.left_over.is_none() {
            let previous = taken
                .iter()
                .rev()
                .find(|block| block.prompt_id != prompt_id && !block.rows.is_empty());
            self.left_over = previous.map(|block| block.rows.clone());
            self.left_over_text = previous.and_then(|block| block.completed.clone());
            // A block's rows are the ones before the last, which was still being written: the screen shows one more.
            self.left_over_slack = usize::from(self.left_over.is_some());
        }
    }

    /// The rows the current block has taken are kept, and it is marked done when it was handed over.
    fn keep_block(&mut self, done: bool) {
        let (Some(context_file), Some(prompt_id), Some(block)) = (
            self.context_file.as_deref(),
            self.prompt_id.as_deref(),
            self.block.as_mut(),
        ) else {
            return;
        };
        if !done && block.rows.len() == block.kept {
            return;
        }
        block.kept = block.rows.len();
        if !block.rows.is_empty() {
            super::reader_state::note_block(
                context_file,
                &block.merge_key_suffix,
                prompt_id,
                &block.rows,
                done,
            );
        }
    }

    fn new(rows: u16, cols: u16) -> Self {
        Self {
            screen: vt100::Parser::new(rows, cols, 0),
            cols: usize::from(cols),
            touched: std::time::Instant::now(),
            prompt_id: None,
            block: None,
            streamed: VecDeque::new(),
            finished: VecDeque::new(),
            completed: VecDeque::new(),
            reader: next_reader(),
            blocks_started: 0,
            previous_rows: HashMap::new(),
            left_over: None,
            left_over_slack: 0,
            left_over_text: None,
            unfinished: None,
            context_file: None,
            recalled: false,
        }
    }

    fn read(&mut self, prompt_id: Option<&str>, bytes: &[u8]) -> Vec<LiveDelta> {
        self.touched = std::time::Instant::now();
        if self.prompt_id.as_deref() != prompt_id {
            // Judged before this read's bytes: they may already draw the new reply.
            let showing = reply_rows(&screen_rows(self.screen.screen()), false).1;
            self.left_over =
                (!showing.is_empty()).then(|| showing.into_iter().map(|row| row.text).collect());
            self.left_over_slack = 0;
            self.left_over_text = None;
            self.completed.clear();
            self.block = None;
            self.streamed.clear();
            self.finished.clear();
            self.previous_rows.clear();
            self.prompt_id = prompt_id.map(str::to_string);
        }
        if !self.recalled && prompt_id.is_some() {
            self.recalled = true;
            self.recall_taken_blocks();
        }
        self.screen.process(bytes);
        if prompt_id.is_none() {
            return Vec::new();
        }
        let delta = self.extract();
        self.keep_block(false);
        delta.into_iter().collect()
    }

    fn extract(&mut self) -> Option<LiveDelta> {
        let (bullet_visible, rows) =
            reply_rows(&screen_rows(self.screen.screen()), self.block.is_some());
        let mut shown: HashMap<String, usize> = HashMap::new();
        for row in &rows {
            *shown.entry(row.text.clone()).or_default() += 1;
        }
        let confirmed = std::mem::replace(&mut self.previous_rows, shown);
        if let Some(left_over) = &self.left_over {
            if repeats(&rows, left_over, self.left_over_slack)
                || self
                    .left_over_text
                    .as_deref()
                    .is_some_and(|text| grown_from(&rows, left_over, text))
            {
                return None;
            }
            self.left_over = None;
            self.left_over_slack = 0;
            self.left_over_text = None;
        }
        if rows.is_empty() {
            return None;
        }
        // The last row may still be written; every row before it is done.
        let done = &rows[..rows.len() - 1];
        if bullet_visible {
            // The block ends when the transcript has its text (`adopt`) or when the
            // screen opens another one. The rows of a block being written are
            // wrapped again as it grows, so they are not compared with the taken ones.
            let block_head = self.block.as_ref().and_then(|block| block.rows.first());
            if block_head.is_some_and(|head| *head != rows[0].text) {
                self.hand_block_over();
            }
            if self.block.is_none() {
                if done.is_empty()
                    || self
                        .finished
                        .iter()
                        .any(|rows_taken| repeats(&rows, rows_taken, 1))
                    || self
                        .completed
                        .iter()
                        .any(|(taken, text)| grown_from(&rows, taken, text))
                    || !confirmed_row(&confirmed, &rows, 0)
                {
                    return None;
                }
                self.blocks_started += 1;
                self.block = Some(Block {
                    merge_key_suffix: self.block_key(self.blocks_started),
                    rows: Vec::new(),
                    last_width: 0,
                    text: String::new(),
                    shown: 0,
                    kept: 0,
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
        for (index, row) in done.iter().enumerate().skip(start) {
            if row.text.is_empty() {
                block.rows.push(String::new());
                continue;
            }
            if !confirmed_row(&confirmed, &rows, index) {
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
        let first = block.text.is_empty();
        if first {
            delta = delta.trim_start().to_string();
        }
        let before = block.text.len();
        block.text.push_str(&delta);
        if first {
            // A reader lost in the middle of a block, and this block drawn again from its start: go on in the row
            // that was begun, and send only what clients do not hold yet. The offer is for this first piece only.
            if let Some(earlier) = self.unfinished.take() {
                if !earlier.text.is_empty()
                    && (earlier.text.starts_with(&block.text)
                        || block.text.starts_with(&earlier.text))
                {
                    block.merge_key_suffix = earlier.merge_key_suffix;
                    block.shown = earlier.text.len();
                }
            }
        }
        if block.text.len() <= block.shown {
            return None;
        }
        let sent_from = before.max(block.shown);
        Some(LiveDelta {
            merge_key_suffix: block.merge_key_suffix.clone(),
            text: block
                .text
                .get(sent_from..)
                .unwrap_or(&block.text)
                .to_string(),
            text_so_far: block.text.clone(),
        })
    }

    fn hand_block_over(&mut self) {
        let Some(block) = self.block.take() else {
            return;
        };
        if !block.rows.is_empty() {
            if let (Some(context_file), Some(prompt_id)) =
                (self.context_file.as_deref(), self.prompt_id.as_deref())
            {
                super::reader_state::note_block(
                    context_file,
                    &block.merge_key_suffix,
                    prompt_id,
                    &block.rows,
                    true,
                );
            }
            self.finished.push_back(block.rows.clone());
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
            merge_key_suffix: streamed.merge_key_suffix,
        }
    }
}

/// Whether `drawn` is the start of `markdown` as the terminal shows it: the
/// letters and digits of what was drawn come in order, close together, at
/// the start of the markdown, which the terminal shows without its markup.
/// Text drawn without a letter (the `};` row of a code block) is matched by
/// its symbols instead.
pub(super) fn drawn_from(drawn: &str, markdown: &str) -> bool {
    /// Characters of markup (a code fence and its language) the terminal leaves out.
    const MARKUP: usize = 16;
    let significant = |text: &str, letters: bool| {
        text.chars()
            .filter(|c| {
                if letters {
                    c.is_alphanumeric()
                } else {
                    !c.is_whitespace()
                }
            })
            .flat_map(char::to_lowercase)
            .collect::<Vec<_>>()
    };
    let letters = drawn.chars().any(char::is_alphanumeric);
    let (drawn, source) = (significant(drawn, letters), significant(markdown, letters));
    if drawn.is_empty() {
        return false;
    }
    let mut position = 0;
    for character in &drawn {
        let Some(found) = source[position..].iter().position(|c| c == character) else {
            return false;
        };
        position += found + 1;
    }
    position <= drawn.len() + drawn.len() / 2 + MARKUP
}

/// Whether the rows on the screen are the `earlier` rows drawn again: the same
/// rows, with up to `slack` more at the end (the last row of a block is not
/// taken) and nothing taken from the middle. Rows that differ are another
/// text, even when it opens with the same line.
fn repeats(rows: &[Row], earlier: &[String], slack: usize) -> bool {
    rows.len() <= earlier.len().saturating_add(slack)
        && rows
            .iter()
            .zip(earlier)
            .all(|(row, earlier)| row.text == *earlier)
}

/// The rows of the block on the screen as one text, a row to a line.
fn drawn_text(rows: &[Row]) -> String {
    rows.iter()
        .map(|row| row.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether the screen shows a block completed by `text` whose first rows the reader had taken: those rows exactly (a
/// code symbol is a symbol), then more, drawn from the rendered Markdown with literal symbols intact.
fn grown_from(rows: &[Row], taken: &[String], text: &str) -> bool {
    !taken.is_empty()
        && rows.len() >= taken.len()
        && rows
            .iter()
            .zip(taken)
            .all(|(row, taken)| row.text == *taken)
        && rendered_prefix(rows, text)
}

/// Markdown syntax is omitted by the terminal, but punctuation inside its text and code is not.
/// Only whitespace may change when the terminal wraps rows; a changed operator starts another block.
fn rendered_prefix(rows: &[Row], markdown: &str) -> bool {
    let mut drawn = String::new();
    for (index, row) in rows.iter().enumerate() {
        drawn.push_str(&row.text);
        drawn.push_str(&row.trailing);
        if index + 1 < rows.len() {
            // A full terminal row may continue one logical code line. Keep that boundary
            // distinct from a short row's newline, without discarding code's spaces.
            drawn.push(if row.width >= usize::from(SCREEN_COLS) {
                '\r'
            } else {
                '\n'
            });
        }
    }
    rendered_text_prefix(&drawn, markdown)
}

fn rendered_text_prefix(mut drawn: &str, markdown: &str) -> bool {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};

    fn take(drawn: &mut &str, character: char, code: bool) -> bool {
        if drawn.is_empty() {
            return true;
        }
        if character == '\n' && code {
            if let Some(tail) = drawn.strip_prefix(['\n', '\r']) {
                *drawn = tail;
                return true;
            }
            return false;
        }
        // Prose wrapping can replace a word separator; code wrapping only inserts a
        // boundary at a full row. Neither permits a space inside a row to disappear.
        if !code && character.is_whitespace() {
            if let Some(first) = drawn.chars().next().filter(|c| c.is_whitespace()) {
                *drawn = &drawn[first.len_utf8()..];
                return true;
            }
            return false;
        }
        *drawn = drawn.trim_start_matches('\r');
        if drawn.is_empty() {
            return true;
        }
        if let Some(tail) = drawn.strip_prefix(character) {
            *drawn = tail;
            true
        } else {
            false
        }
    }

    if drawn.trim().is_empty() {
        return false;
    }
    let mut lists = Vec::new();
    let mut code_block = false;
    let mut boundary = true;
    for event in Parser::new(markdown) {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                code_block = true;
                boundary = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                code_block = false;
                boundary = true;
            }
            Event::Start(Tag::Paragraph | Tag::Heading { .. } | Tag::BlockQuote(_)) => {
                boundary = true
            }
            Event::End(
                TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item | TagEnd::BlockQuote(_),
            ) => boundary = true,
            Event::Start(Tag::List(first)) => lists.push(first),
            Event::End(TagEnd::List(_)) => {
                lists.pop();
                boundary = true;
            }
            Event::Start(Tag::Item) => {
                drawn = drawn.trim_start();
                let markers = match lists.last_mut() {
                    Some(Some(number)) => {
                        let marker = format!("{number}.");
                        *number += 1;
                        vec![marker]
                    }
                    _ => vec!["•".to_string(), "-".to_string(), "*".to_string()],
                };
                for marker in markers {
                    if let Some(tail) = drawn.strip_prefix(&marker) {
                        if let Some(space) = tail.chars().next().filter(|c| c.is_whitespace()) {
                            drawn = &tail[space.len_utf8()..];
                            break;
                        }
                    }
                }
                boundary = true;
            }
            Event::SoftBreak | Event::HardBreak => {
                if !take(&mut drawn, ' ', false) {
                    return false;
                }
            }
            Event::Text(text) => {
                if boundary {
                    drawn = if code_block {
                        drawn.trim_start_matches(['\n', '\r'])
                    } else {
                        drawn.trim_start()
                    };
                    boundary = false;
                }
                for character in text.chars() {
                    if drawn.is_empty() {
                        return true;
                    }
                    if !take(&mut drawn, character, code_block) {
                        return false;
                    }
                }
            }
            Event::Code(text) => {
                if boundary {
                    drawn = drawn.trim_start_matches(['\n', '\r']);
                    boundary = false;
                }
                for character in text.chars() {
                    if drawn.is_empty() {
                        return true;
                    }
                    if !take(&mut drawn, character, true) {
                        return false;
                    }
                }
            }
            _ => {}
        }
    }
    drawn.trim().is_empty()
}

/// Whether the previous read showed row `index` too: a row repeated on the
/// screen counts as often as it was shown, so a new copy of a line is not
/// taken for the old one.
fn confirmed_row(confirmed: &HashMap<String, usize>, rows: &[Row], index: usize) -> bool {
    let text = &rows[index].text;
    let nth = rows[..=index]
        .iter()
        .filter(|row| row.text == *text)
        .count();
    confirmed.get(text).is_some_and(|shown| *shown >= nth)
}

fn screen_rows(screen: &vt100::Screen) -> Vec<Row> {
    let (rows, cols) = screen.size();
    (0..rows)
        .map(|row| {
            let mut text = String::new();
            let mut width = 0;
            let mut printed_end = 0;
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
                printed_end = text.len();
                width = usize::from(col) + if cell.is_wide() { 2 } else { 1 };
            }
            Row {
                trailing: text[text.trim_end().len()..printed_end].to_string(),
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
        let trailing = if row.text.is_empty() {
            row.trailing
                .strip_prefix(INDENT)
                .unwrap_or(&row.trailing)
                .to_string()
        } else {
            row.trailing.clone()
        };
        reply.push(Row {
            text,
            trailing,
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
