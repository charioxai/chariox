//! Room terminals: kernel-owned shells that users and agents share.

mod driver;
mod output_log;
mod plain_text;
mod screen;
mod service;
mod store;
mod terminal;
mod types;
mod watch;

#[cfg(all(test, unix))]
mod tests;

pub use service::{OpenRoomTerminal, RoomTerminalService};
pub use types::*;
