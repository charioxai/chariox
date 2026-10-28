use crate::ipc::Ipc;
use serde_json::json;
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::AsyncWrite;
use wasmtime_wasi::cli::{IsTerminal, StdoutStream};

#[derive(Clone)]
pub(super) struct Output {
    ipc: Ipc,
    level: &'static str,
}
impl Output {
    pub(super) fn new(ipc: Ipc, level: &'static str) -> Self {
        Self { ipc, level }
    }
}
impl IsTerminal for Output {
    fn is_terminal(&self) -> bool {
        false
    }
}
impl StdoutStream for Output {
    fn async_stream(&self) -> Box<dyn AsyncWrite + Send + Sync> {
        Box::new(self.clone())
    }
}
impl AsyncWrite for Output {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let count = bytes.len().min(4096);
        if count != 0 {
            self.ipc.cleanup(
                "log.write",
                json!({"level":self.level,
                "message":String::from_utf8_lossy(&bytes[..count]), "fields":{"source":"wasi"}}),
            );
        }
        Poll::Ready(Ok(count))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
