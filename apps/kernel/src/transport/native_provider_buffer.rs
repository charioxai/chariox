//! MP-08 / MP-10 / MP-11: byte-bounded official catalog frames, not provider payload logs.
use super::native_provider_socket::error;
use crate::error::DaemonError;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(crate) const OUTPUT_FRAME_BYTES: usize = 16 * 1024 * 1024;
const CONNECTION_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
const KERNEL_OUTPUT_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct NativeOutputBudget(Arc<Semaphore>);
impl Default for NativeOutputBudget {
    fn default() -> Self {
        Self(Arc::new(Semaphore::new(KERNEL_OUTPUT_BYTES)))
    }
}
impl NativeOutputBudget {
    pub(crate) fn connection(&self) -> NativeConnectionOutputBudget {
        NativeConnectionOutputBudget {
            kernel: self.0.clone(),
            connection: Arc::new(Semaphore::new(CONNECTION_OUTPUT_BYTES)),
        }
    }
}
#[derive(Clone)]
pub(crate) struct NativeConnectionOutputBudget {
    kernel: Arc<Semaphore>,
    connection: Arc<Semaphore>,
}
pub(crate) struct NativeBufferedFrame {
    pub(crate) data: String,
    _kernel: OwnedSemaphorePermit,
    _connection: OwnedSemaphorePermit,
}
impl NativeConnectionOutputBudget {
    pub(crate) fn reserve(&self, data: String) -> Result<NativeBufferedFrame, DaemonError> {
        if data.len() > OUTPUT_FRAME_BYTES {
            return Err(error("native output frame exceeds 16 MiB"));
        }
        let bytes = data.len() as u32;
        let kernel = self
            .kernel
            .clone()
            .try_acquire_many_owned(bytes)
            .map_err(|_| error("native kernel output byte budget exceeded"))?;
        let connection = self
            .connection
            .clone()
            .try_acquire_many_owned(bytes)
            .map_err(|_| error("native connection output byte budget exceeded"))?;
        Ok(NativeBufferedFrame {
            data,
            _kernel: kernel,
            _connection: connection,
        })
    }
}
