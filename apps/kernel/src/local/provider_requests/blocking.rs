use crate::error::DaemonError;
use tokio::runtime::Handle;
use tokio::runtime::Runtime;

pub(super) fn block_on_relay_query<F, T>(future: F) -> Result<T, DaemonError>
where
    F: std::future::Future<Output = Result<T, DaemonError>>,
{
    if let Ok(handle) = Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(future))
    } else {
        Runtime::new()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "create relay discovery runtime",
                message: error.to_string(),
            })?
            .block_on(future)
    }
}
