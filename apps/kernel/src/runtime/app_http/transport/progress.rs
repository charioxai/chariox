use std::{
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    time::Instant,
};

#[derive(Clone)]
pub(super) struct Progress(Arc<Mutex<Instant>>);
impl Progress {
    pub(super) fn new() -> Self {
        Self(Arc::new(Mutex::new(Instant::now())))
    }
    fn update(&self) {
        *self.0.lock().unwrap() = Instant::now();
    }
    pub(super) async fn inactive(&self, limit: Duration) {
        loop {
            let deadline = *self.0.lock().unwrap() + limit;
            tokio::time::sleep_until(deadline).await;
            if *self.0.lock().unwrap() + limit <= Instant::now() {
                return;
            }
        }
    }
}
pub(super) struct ObservedIo<T> {
    io: T,
    progress: Progress,
}
impl<T> ObservedIo<T> {
    pub(super) fn new(io: T, progress: Progress) -> Self {
        Self { io, progress }
    }
}
impl<T: AsyncRead + Unpin> AsyncRead for ObservedIo<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.io).poll_read(cx, buf);
        if matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            self.progress.update();
        }
        result
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for ObservedIo<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.io).poll_write(cx, buf);
        if matches!(result, Poll::Ready(Ok(n)) if n > 0) {
            self.progress.update();
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}
