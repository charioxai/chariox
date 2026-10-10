//! Interrupt blocking helper I/O when shared grant/action authority is revoked.
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub(super) struct RequestFence {
    done: Sender<()>,
    worker: Option<JoinHandle<()>>,
}
impl RequestFence {
    pub(super) fn watch(
        revoked: impl Fn() -> bool + Send + 'static,
        fence: impl FnOnce() + Send + 'static,
    ) -> Self {
        let (done, receiver) = mpsc::channel();
        let worker = thread::spawn(move || loop {
            if revoked() {
                fence();
                return;
            }
            match receiver.recv_timeout(Duration::from_millis(5)) {
                Err(RecvTimeoutError::Timeout) => {}
                _ => return,
            }
        });
        Self {
            done,
            worker: Some(worker),
        }
    }
}
impl Drop for RequestFence {
    fn drop(&mut self) {
        let _ = self.done.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    #[test]
    fn revoke_interrupts_blocked_socket_before_request_completion() {
        use std::io::Read;
        use std::os::unix::net::UnixStream;
        let (socket, mut peer) = UnixStream::pair().unwrap();
        let revoked = Arc::new(AtomicBool::new(false));
        let flag = revoked.clone();
        let guard = RequestFence::watch(
            move || flag.load(Ordering::Acquire),
            move || socket.shutdown(std::net::Shutdown::Both).unwrap(),
        );
        let (sent, received) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut byte = [0];
            sent.send(peer.read(&mut byte)).unwrap();
        });
        revoked.store(true, Ordering::Release);
        assert_eq!(
            received
                .recv_timeout(Duration::from_millis(250))
                .unwrap()
                .unwrap(),
            0
        );
        reader.join().unwrap();
        drop(guard);
    }
    #[test]
    fn completed_request_does_not_later_fence_a_new_epoch() {
        let revoked = Arc::new(AtomicBool::new(false));
        let flag = revoked.clone();
        let fenced = Arc::new(AtomicBool::new(false));
        let effect = fenced.clone();
        drop(RequestFence::watch(
            move || flag.load(Ordering::Acquire),
            move || effect.store(true, Ordering::Release),
        ));
        revoked.store(true, Ordering::Release);
        assert!(!fenced.load(Ordering::Acquire));
    }
}
