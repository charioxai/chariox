//! A launch held before start can run on another owned kernel instead.
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use super::{ExecOutput, LaunchIntent};
use crate::error::DaemonError;

pub(crate) type PlacementFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, DaemonError>> + Send + 'a>>;

/// The binary exec operations of one other kernel, reached with the holding
/// agent's own gateway grant. Implementations forward the existing exec
/// operations; they add no authority of their own.
pub(crate) trait ExecPlacement: Send + Sync {
    /// Starts the launch there exactly once and returns the target's exec id.
    /// `attributed_agent` names the holding agent as the target must show it.
    fn open<'a>(
        &'a self,
        intent: &'a LaunchIntent,
        attributed_agent: Option<String>,
    ) -> PlacementFuture<'a, String>;
    fn write<'a>(
        &'a self,
        exec_id: &'a str,
        offset: u64,
        data_base64: &'a str,
    ) -> PlacementFuture<'a, u64>;
    fn close_stdin<'a>(&'a self, exec_id: &'a str, offset: u64) -> PlacementFuture<'a, u64>;
    fn read<'a>(
        &'a self,
        exec_id: &'a str,
        after_cursor: u64,
        wait: Duration,
    ) -> PlacementFuture<'a, ExecOutput>;
    fn cancel<'a>(&'a self, exec_id: &'a str) -> PlacementFuture<'a, ()>;
}
