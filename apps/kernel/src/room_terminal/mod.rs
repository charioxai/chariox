//! Room terminals: kernel-owned shells that users and agents share.

mod driver;
mod events;
mod output_log;
mod plain_text;
mod pump;
mod screen;
mod service;
mod shell;
mod store;
mod terminal;
mod types;

#[cfg(all(test, unix))]
mod tests;

pub use events::{CompletionEvents, EventCoreNotWired, WatchOwner};
pub use service::{OpenRoomTerminal, RoomTerminalService, MAX_WAIT};
pub use types::*;
