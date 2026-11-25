//! MP-08 / MP-10: what the live reply reader must not forget when it is lost. The reader lives in memory, and a kernel
//! restart or the run registry's eviction ends it, while Claude's screen still shows what the reader had taken. The rows
//! of every block the reader streams are kept next to the run's context file (as the rows still waiting for their
//! transcript text are, see `awaiting_rows`), and a block is marked done when it was handed over or when its transcript
//! text completed it, by the reader or without it. The reader that takes over does not stream the done blocks again.
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const BLOCKS_FILE: &str = "live-reader-blocks";
/// Blocks kept, as many as the reader keeps after handing them over.
const KEPT: usize = 16;

/// The rows taken of one block, under the merge key suffix it is streamed as, for the prompt it was streamed under.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Taken {
    pub(super) key: String,
    pub(super) prompt_id: String,
    pub(super) rows: Vec<String>,
    /// The block was handed over, or its transcript text completed it.
    pub(super) done: bool,
    /// The transcript text that completed it: the block may have grown beyond the rows the reader had taken.
    #[serde(default)]
    pub(super) completed: Option<String>,
}

fn blocks_file(context_file: &str) -> PathBuf {
    std::path::Path::new(context_file).with_file_name(BLOCKS_FILE)
}

/// The blocks streamed so far, oldest first.
pub(super) fn recall(context_file: &str) -> Vec<Taken> {
    fs::read_to_string(blocks_file(context_file))
        .map(|raw| {
            raw.lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn write(context_file: &str, blocks: Vec<Taken>) {
    let excess = blocks.len().saturating_sub(KEPT);
    let lines: Vec<String> = blocks
        .into_iter()
        .skip(excess)
        .filter_map(|block| serde_json::to_string(&block).ok())
        .collect();
    let _ = fs::write(blocks_file(context_file), lines.join("\n") + "\n");
}

/// The block under `key` has these rows so far; `done` when it was handed over.
pub(super) fn note_block(
    context_file: &str,
    key: &str,
    prompt_id: &str,
    rows: &[String],
    done: bool,
) {
    let mut blocks = recall(context_file);
    let taken = Taken {
        key: key.to_string(),
        prompt_id: prompt_id.to_string(),
        rows: rows.to_vec(),
        done,
        completed: None,
    };
    match blocks.iter_mut().find(|block| block.key == key) {
        Some(block) if *block == taken => return,
        Some(block) => {
            *block = Taken {
                done: done || block.done,
                ..taken
            }
        }
        None => blocks.push(taken),
    }
    write(context_file, blocks);
}

/// The transcript text completed the block under `key`, whether or not a reader streamed it.
pub(super) fn mark_done(context_file: &str, key: &str, completed: Option<&str>) {
    let mut blocks = recall(context_file);
    if let Some(block) = blocks.iter_mut().find(|block| block.key == key) {
        block.done = true;
        block.completed = completed.map(str::to_string);
        write(context_file, blocks);
    }
}
