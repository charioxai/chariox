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
}

#[cfg(unix)]
pub(super) struct CodexOutputReadiness {
    shared: Arc<(Mutex<WatchState>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
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
        let thread = std::thread::Builder::new()
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
        Some(Self {
            shared,
            thread: Some(thread),
        })
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
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
