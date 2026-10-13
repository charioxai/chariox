//! MP-08 / MP-10: observe socket readiness without consuming or parsing provider
//! messages outside the actor. Acknowledgement prevents a readable socket from
//! spinning while its actor is queued or servicing another command.
#[cfg(unix)]
use std::sync::{Arc, Condvar, Mutex};

#[cfg(unix)]
#[derive(Default)]
struct WatchState {
    stopped: bool,
    polled: u64,
    #[cfg(test)]
    entered_poll: bool,
}

#[cfg(unix)]
pub(super) struct CodexOutputReadiness {
    shared: Arc<(Mutex<WatchState>, Condvar)>,
}

#[cfg(unix)]
impl CodexOutputReadiness {
    pub(super) fn start(
        socket: &crate::provider::CodexSocket,
        notifier: crate::provider::output_ready::ProviderEventNotifier,
    ) -> Option<Self> {
        use std::os::fd::AsRawFd;
        let tokio_tungstenite::tungstenite::stream::MaybeTlsStream::Plain(stream) =
            socket.get_ref()
        else {
            return None;
        };
        let stream = stream.try_clone().ok()?;
        let shared = Arc::new((Mutex::new(WatchState::default()), Condvar::new()));
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("chariox-codex-ready".into())
            .spawn(move || {
                loop {
                    let polled = {
                        let state = worker.0.lock().expect("codex readiness poisoned");
                        if state.stopped {
                            break;
                        }
                        state.polled
                    };
                    let mut fd = libc::pollfd {
                        fd: stream.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    // The clone keeps the fd alive. This only observes readiness;
                    // socket reads/writes and their timeout flags remain actor-owned.
                    #[cfg(test)]
                    {
                        worker.0.lock().unwrap().entered_poll = true;
                        worker.1.notify_all();
                    }
                    let ready = unsafe { libc::poll(&mut fd, 1, 500) };
                    if ready < 0 {
                        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                        {
                            continue;
                        }
                        break;
                    }
                    if ready == 0 {
                        continue;
                    }
                    notifier.notify();
                    let mut state = worker.0.lock().expect("codex readiness poisoned");
                    while !state.stopped && state.polled == polled {
                        state = worker.1.wait(state).expect("codex readiness poisoned");
                    }
                    if state.stopped {
                        break;
                    }
                }
            })
            .ok()?;
        // Detach: runtime removal can hold the provider-service lock. The
        // stop flag/condvar still bound this thread's lifetime to one poll.
        Some(Self { shared })
    }
    pub(super) fn polled(&self) {
        let mut state = self.shared.0.lock().expect("codex readiness poisoned");
        state.polled = state.polled.wrapping_add(1);
        self.shared.1.notify_all();
    }
}
#[cfg(unix)]
impl Drop for CodexOutputReadiness {
    fn drop(&mut self) {
        self.shared
            .0
            .lock()
            .expect("codex readiness poisoned")
            .stopped = true;
        self.shared.1.notify_all();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::time::{Duration, Instant};
    use tokio_tungstenite::tungstenite::{protocol::Role, stream::MaybeTlsStream, WebSocket};

    // MP-08 / MP-10: reviewer #985 finding 3. An actual idle socket
    // puts the watcher in poll before Drop is measured.
    #[test]
    fn panes_stream_review_watcher_drop_does_not_wait_on_socket() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (peer, _) = listener.accept().unwrap();
        let socket = WebSocket::from_raw_socket(MaybeTlsStream::Plain(stream), Role::Client, None);
        let watcher = CodexOutputReadiness::start(&socket, Default::default()).unwrap();
        let shared = watcher.shared.clone();
        {
            let state = shared.0.lock().unwrap();
            let (state, timeout) = shared
                .1
                .wait_timeout_while(state, Duration::from_secs(2), |s| !s.entered_poll)
                .unwrap();
            assert!(!timeout.timed_out() && state.entered_poll);
        }
        let started = Instant::now();
        drop(watcher);
        let elapsed = started.elapsed();
        drop(peer);
        let deadline = Instant::now() + Duration::from_secs(2);
        while Arc::strong_count(&shared) != 1 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(Arc::strong_count(&shared), 1, "stopped watcher must exit");
        assert!(
            elapsed < Duration::from_millis(50),
            "drop blocked for {elapsed:?}"
        );
    }
}
