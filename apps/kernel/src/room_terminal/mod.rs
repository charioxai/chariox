//! Room terminals: kernel-owned shells that users and agents share.

mod driver;
mod events;
mod managed_work;
mod output_log;
mod plain_text;
mod pump;
mod screen;
mod service;
mod session_sweep;
mod shell;
mod shell_marks;
mod store;
mod terminal;
mod types;

#[cfg(all(test, unix))]
mod tests;

pub use events::{CompletionEvents, WatchOwner};
pub use service::{OpenRoomTerminal, RoomTerminalService, MAX_WAIT};
pub use types::*;
