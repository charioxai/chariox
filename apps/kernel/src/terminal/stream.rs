use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::history::{SessionHistoryEntrySource, SessionHistoryExternalObservation};
use crate::session::{unix_epoch_ms, PromptOrigin};

mod json_size;
mod records;
mod service;
mod store;

#[cfg(test)]
mod tests;

pub use records::{
    AssistantMessageCompletionRecord, RuntimeNoticeRecord, TerminalInputRecord,
    TerminalOutputAppend, TerminalOutputExternalObservationMetadata, TerminalOutputKind,
    TerminalOutputRecord, TerminalStreamHealthSnapshot, TerminalStreamHealthStore,
};
pub use service::TerminalStreamService;
pub use store::TerminalStreamStore;

const DEFAULT_PENDING_OUTPUT_RECORD_LIMIT_PER_ATTACHMENT: usize = 4096;
const DEFAULT_OUTPUT_COALESCE_BYTE_LIMIT: usize = 16 * 1024;
const DEFAULT_OUTPUT_DRAIN_JSON_LIMIT: usize = 128 * 1024;
