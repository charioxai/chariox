use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex, OnceLock};

const PROMPT_ID_RESERVATION_BLOCK: u64 = 4_096;

#[derive(Debug, Clone)]
pub struct PromptIdAllocator {
    inner: Arc<PromptIdAllocatorInner>,
}

#[derive(Debug)]
struct PromptIdAllocatorInner {
    state: Mutex<PromptIdAllocatorState>,
    counter: Option<PromptIdCounter>,
}

#[derive(Debug)]
struct PromptIdAllocatorState {
    last_issued: u64,
    reserved_until: u64,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct DurablePromptIdCounter {
    high_water_prompt_id: u64,
}

impl Default for PromptIdAllocator {
    fn default() -> Self {
        Self {
            inner: Arc::new(PromptIdAllocatorInner {
                state: Mutex::new(PromptIdAllocatorState {
                    last_issued: 0,
                    reserved_until: u64::MAX,
                }),
                counter: None,
            }),
        }
    }
}

impl PromptIdAllocator {
    pub(crate) fn persistent(counter_path: PathBuf) -> Self {
        Self {
            inner: Arc::new(PromptIdAllocatorInner {
                state: Mutex::new(PromptIdAllocatorState {
                    last_issued: 0,
                    reserved_until: 0,
                }),
                counter: Some(PromptIdCounter {
                    path: counter_path,
                    writer: OnceLock::new(),
                }),
            }),
        }
    }

    pub(crate) fn next_prompt_id(&self) -> String {
        if let Ok(mut state) = self.inner.state.try_lock() {
            if state.last_issued < state.reserved_until {
                return issue_reserved_prompt_id(&mut state);
            }
        }

        // Both a new reservation and waiting for another caller's reservation
        // may take as long as the disk. Keep those waits off Tokio workers.
        let reserve = || self.reserve_next_prompt_id();
        if tokio::runtime::Handle::try_current().is_ok_and(|handle| {
            handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread
        }) {
            tokio::task::block_in_place(reserve)
        } else {
            reserve()
        }
    }

    fn reserve_next_prompt_id(&self) -> String {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("prompt id allocator lock poisoned");
        if state.last_issued >= state.reserved_until {
            if let Some(counter) = &self.inner.counter {
                match counter.reserve(state.last_issued) {
                    Ok((first, reserved_until)) => {
                        state.last_issued = first.saturating_sub(1);
                        state.reserved_until = reserved_until;
                    }
                    Err(error) => {
                        let fallback = super::unix_epoch_ms()
                            .saturating_mul(1_000_000)
                            .max(state.last_issued.saturating_add(1));
                        crate::logging::warn_with_fields(
                            "durable_state.prompt_id",
                            "failed to reserve durable prompt id block; using process-local high-water fallback",
                            serde_json::json!({
                                "counter_path": counter.path.display().to_string(),
                                "error": error.to_string(),
                                "fallback_prompt_number": fallback,
                            }),
                        );
                        state.last_issued = fallback.saturating_sub(1);
                        state.reserved_until = u64::MAX;
                    }
                }
            }
        }
        issue_reserved_prompt_id(&mut state)
    }

    #[cfg(test)]
    pub(crate) fn observe_prompt_id(&self, prompt_id: &str) {
        if let Some(number) = prompt_id_number(prompt_id) {
            self.advance_to_at_least(number);
        }
    }

    #[cfg(test)]
    fn advance_to_at_least(&self, number: u64) {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("prompt id allocator lock poisoned");
        state.last_issued = state.last_issued.max(number);
    }
}

fn issue_reserved_prompt_id(state: &mut PromptIdAllocatorState) -> String {
    state.last_issued = state.last_issued.saturating_add(1);
    format!("prompt-{}", state.last_issued)
}

#[derive(Debug)]
struct PromptIdCounter {
    path: PathBuf,
    writer: OnceLock<std::io::Result<mpsc::SyncSender<Reservation>>>,
}

struct Reservation {
    minimum: u64,
    reply: mpsc::Sender<std::io::Result<(u64, u64)>>,
}

impl PromptIdCounter {
    fn reserve(&self, minimum: u64) -> std::io::Result<(u64, u64)> {
        let writer = self.writer.get_or_init(|| self.start_writer());
        let writer = writer
            .as_ref()
            .map_err(|error| std::io::Error::new(error.kind(), error.to_string()))?;
        let (reply, answer) = mpsc::channel();
        writer
            .try_send(Reservation { minimum, reply })
            .map_err(|_| std::io::Error::other("prompt id writer cannot accept a reservation"))?;
        answer
            .recv()
            .map_err(|_| std::io::Error::other("prompt id writer stopped"))?
    }

    fn start_writer(&self) -> std::io::Result<mpsc::SyncSender<Reservation>> {
        let (sender, requests) = mpsc::sync_channel::<Reservation>(1);
        let path = self.path.clone();
        std::thread::Builder::new()
            .name("chariox-prompt-id".into())
            .spawn(move || {
                while let Ok(request) = requests.recv() {
                    let result = reserve_prompt_id_block(&path, request.minimum);
                    let _ = request.reply.send(result);
                }
            })?;
        Ok(sender)
    }
}

fn reserve_prompt_id_block(path: &Path, minimum: u64) -> std::io::Result<(u64, u64)> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let lock_path = path.with_extension("lock");
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)?;
    lock_file_exclusive(&lock_file)?;
    let current = match std::fs::read(path) {
        Ok(payload) => serde_json::from_slice::<DurablePromptIdCounter>(&payload)
            .map(|counter| counter.high_water_prompt_id)
            .map_err(std::io::Error::other)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error),
    };
    let baseline = current.max(minimum).max(super::unix_epoch_ms());
    let reserved_until = baseline
        .checked_add(PROMPT_ID_RESERVATION_BLOCK)
        .ok_or_else(|| std::io::Error::other("prompt id reservation overflow"))?;
    let payload = serde_json::to_vec(&DurablePromptIdCounter {
        high_water_prompt_id: reserved_until,
    })
    .map_err(std::io::Error::other)?;
    let temporary = path.with_extension(format!("json.tmp-{}", std::process::id()));
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&payload)?;
        file.sync_all()?;
    }
    std::fs::rename(&temporary, path)?;
    Ok((baseline.saturating_add(1), reserved_until))
}

#[cfg(unix)]
fn lock_file_exclusive(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    // Release the reservation lock when this transaction's descriptor closes.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn lock_file_exclusive(_file: &std::fs::File) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "durable prompt id allocation requires an exclusive file lock",
    ))
}

#[cfg(test)]
fn prompt_id_number(prompt_id: &str) -> Option<u64> {
    prompt_id.strip_prefix("prompt-")?.parse::<u64>().ok()
}

#[cfg(all(test, unix))]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn prompt_id_reservation_wait_does_not_block_the_runtime_worker() {
    use std::time::Duration;

    let root = crate::test_support::TestWorktree::new("prompt-counter-blocked");
    let path = root.path().join("counter.json");
    let lock = std::fs::File::create(path.with_extension("lock")).unwrap();
    lock_file_exclusive(&lock).unwrap();
    let ids = PromptIdAllocator::persistent(path.clone());
    let (pulse, pulsed) = mpsc::channel();
    let release = std::thread::spawn(move || {
        let ran_while_counter_was_locked = pulsed.recv_timeout(Duration::from_secs(2)).is_ok();
        drop(lock);
        ran_while_counter_was_locked
    });
    let (starting, started) = tokio::sync::oneshot::channel();
    let allocation = tokio::spawn(async move {
        starting.send(()).unwrap();
        ids.next_prompt_id()
    });
    started.await.unwrap();
    tokio::spawn(async move {
        let _ = pulse.send(());
    })
    .await
    .unwrap();
    let issued = allocation.await.unwrap();
    assert!(
        release.join().unwrap(),
        "reservation occupied the only runtime worker"
    );
    let checkpoint: DurablePromptIdCounter =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(prompt_id_number(&issued).unwrap() <= checkpoint.high_water_prompt_id);
}
