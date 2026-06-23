use crate::error::DaemonError;
use tokio::runtime::Builder;
use tokio::runtime::Handle;

const LOCAL_BLOCKING_RUNTIME_THREAD_STACK_SIZE: usize = 32 * 1024 * 1024;

pub(super) fn block_on_relay_query<F, T>(future: F) -> Result<T, DaemonError>
where
    F: std::future::Future<Output = Result<T, DaemonError>>,
{
    if let Ok(handle) = Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(future))
    } else {
        Builder::new_multi_thread()
            .enable_all()
            .thread_stack_size(LOCAL_BLOCKING_RUNTIME_THREAD_STACK_SIZE)
            .build()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "create relay discovery runtime",
                message: error.to_string(),
            })?
            .block_on(future)
    }
}
