//! MP-08 / MP-10 / MP-11: deferred discovery and reaping retain kernel ownership.
use std::collections::BTreeSet;
use std::os::fd::{AsRawFd, OwnedFd};
use std::process::Child;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex, OnceLock,
};
use std::time::Duration;

pub(super) struct Pending {
    pub(super) child: Child,
    pub(super) pipes: Vec<OwnedFd>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(super) descendants: Vec<super::descendants::Descendant>,
    pub(super) cleaned: bool,
}
impl Pending {
    fn cleanup(&mut self) -> std::io::Result<()> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            for pass in 0..3 {
                super::descendants::observe(
                    Some(self.child.id()),
                    &mut self.descendants,
                    pass == 0,
                )?;
                for child in &self.descendants {
                    child.signal(libc::SIGSTOP)?;
                }
            }
            for child in &self.descendants {
                child.signal(libc::SIGKILL)?;
            }
            let fds: Vec<_> = if self.pipes.is_empty() {
                [
                    self.child.stdout.as_ref().map(AsRawFd::as_raw_fd),
                    self.child.stderr.as_ref().map(AsRawFd::as_raw_fd),
                ]
                .into_iter()
                .flatten()
                .collect()
            } else {
                self.pipes.iter().map(AsRawFd::as_raw_fd).collect()
            };
            if !fds.is_empty() && super::pipe_writers_open(&fds)? {
                super::pipe_owners::terminate(&fds)?;
            }
        }
        self.cleaned = true;
        Ok(())
    }
    fn finished(&mut self) -> bool {
        if !self.cleaned && self.cleanup().is_err() {
            // Retain ownership and retry; never drop an incompletely cleaned tree.
            return false;
        }
        match self.child.try_wait() {
            Ok(Some(_)) => {}
            Err(error) if error.raw_os_error() == Some(libc::ECHILD) => {}
            _ => return false,
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            !self.descendants.iter().any(|child| child.alive())
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            true
        }
    }
}
struct Work {
    probe: Pending,
    done: Arc<AtomicBool>,
}
struct Reaper {
    send: mpsc::Sender<Work>,
    owned: Arc<Mutex<BTreeSet<u32>>>,
}
fn reaper() -> &'static Reaper {
    static REAPER: OnceLock<Reaper> = OnceLock::new();
    REAPER.get_or_init(|| {
        let (send, receive) = mpsc::channel::<Work>();
        let owned = Arc::new(Mutex::new(BTreeSet::new()));
        let inventory = owned.clone();
        std::thread::Builder::new()
            .name("probe-reaper".into())
            .spawn(move || {
                let mut pending = Vec::<Work>::new();
                loop {
                    let next = if pending.is_empty() {
                        receive.recv().ok()
                    } else {
                        receive.recv_timeout(Duration::from_millis(25)).ok()
                    };
                    if let Some(work) = next {
                        pending.push(work);
                    }
                    pending.retain_mut(|work| {
                        if !work.probe.finished() {
                            return true;
                        }
                        inventory
                            .lock()
                            .expect("probe inventory")
                            .remove(&work.probe.child.id());
                        work.done.store(true, Ordering::Release);
                        false
                    });
                }
            })
            .expect("start owned probe reaper");
        Reaper { send, owned }
    })
}
pub(super) fn defer(probe: Pending) -> Arc<AtomicBool> {
    let reaper = reaper();
    reaper
        .owned
        .lock()
        .expect("probe inventory")
        .insert(probe.child.id());
    let done = Arc::new(AtomicBool::new(false));
    reaper
        .send
        .send(Work {
            probe,
            done: done.clone(),
        })
        .unwrap_or_else(|_| panic!("owned probe reaper stopped"));
    done
}
#[cfg(test)]
pub(super) fn owns_child(pid: u32) -> bool {
    reaper()
        .owned
        .lock()
        .expect("probe inventory")
        .contains(&pid)
}
