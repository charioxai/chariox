//! Room terminals: kernel-owned shells that users and agents share.

mod driver;
mod events;
pub(crate) mod managed_work;
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

/// The code of a terminal refusal; its message is the service's own sentence.
pub(crate) const REFUSED_CODE: &str = "terminal_refused";

/// Codes whose message the terminal service writes itself.
pub(crate) fn is_public_code(code: &str) -> bool {
    code == REFUSED_CODE || code.starts_with("exec_")
}

/// A refusal in the terminal service's own words, for a caller outside the service whose
/// sentence an outside agent must be able to read (`is_public_code`).
pub(crate) fn refusal(message: String) -> crate::error::DaemonError {
    service::refused(message)
}

#[cfg(test)]
pub(crate) fn refused_for_test(message: &str) -> crate::error::DaemonError {
    service::refused(message.to_string())
}

pub use events::{CompletionEvents, WatchOwner};
pub(crate) use service::KERNEL_SECRET_ENVIRONMENT;
pub use service::{OpenRoomTerminal, RoomTerminalService, MAX_WAIT};
pub use types::*;
