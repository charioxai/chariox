//! MP-08 / MP-10: what the live reply reader must not forget when it is lost. The reader lives in memory, and a kernel
//! restart or the run registry's eviction ends it, while Claude's screen still shows what the reader had taken. The rows
//! of every block the reader handed over are kept next to the run's context file (as the rows still waiting for their
//! transcript text are, see `awaiting_rows`), so the reader that takes over does not stream them again as new rows.
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const BLOCKS_FILE: &str = "live-reader-blocks";
/// Blocks kept, as many as the reader keeps after handing them over.
const KEPT: usize = 16;

/// The rows taken of one block, for the prompt it was streamed under.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Taken {
    pub(super) prompt_id: String,
    pub(super) rows: Vec<String>,
}

fn blocks_file(context_file: &str) -> PathBuf {
    std::path::Path::new(context_file).with_file_name(BLOCKS_FILE)
}

/// The blocks handed over so far, oldest first.
pub(super) fn recall(context_file: &str) -> Vec<Taken> {
    fs::read_to_string(blocks_file(context_file))
        .map(|raw| {
            raw.lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// A block of `prompt_id` was handed over with these rows.
pub(super) fn note_taken(context_file: &str, prompt_id: &str, rows: &[String]) {
    let mut blocks = recall(context_file);
    let taken = Taken {
        prompt_id: prompt_id.to_string(),
        rows: rows.to_vec(),
    };
    if blocks.last() == Some(&taken) {
        return;
    }
    blocks.push(taken);
    let excess = blocks.len().saturating_sub(KEPT);
    let lines: Vec<String> = blocks
        .into_iter()
        .skip(excess)
        .filter_map(|block| serde_json::to_string(&block).ok())
        .collect();
    let _ = fs::write(blocks_file(context_file), lines.join("\n") + "\n");
}
