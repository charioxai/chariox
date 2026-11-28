//! A deterministic admitted-chunk pause for the real write-entry race regression.
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};
use std::time::Duration;
use uuid::Uuid;

struct Hook {
    path: PathBuf,
    id: Uuid,
    reached: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}
static HOOK: Mutex<Option<Hook>> = Mutex::new(None);

pub(super) struct Pause {
    id: Uuid,
    reached: mpsc::Receiver<()>,
    release: mpsc::Sender<()>,
}
impl Pause {
    pub(super) fn wait_until_admitted(&self) {
        self.reached
            .recv_timeout(Duration::from_secs(5))
            .expect("the original chunk is admitted");
    }
}
impl Drop for Pause {
    fn drop(&mut self) {
        let mut hook = HOOK.lock().unwrap();
        if hook.as_ref().is_some_and(|hook| hook.id == self.id) {
            *hook = None;
        }
        let _ = self.release.send(());
    }
}

pub(super) fn pause(path: &Path, id: Uuid) -> Pause {
    let (reached, observed) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let mut hook = HOOK.lock().unwrap();
    assert!(hook.is_none());
    *hook = Some(Hook {
        path: path.to_path_buf(),
        id,
        reached,
        release: released,
    });
    Pause {
        id,
        reached: observed,
        release,
    }
}

pub(super) fn after_admission(path: &Path, id: Option<Uuid>) {
    let hook = {
        let mut hook = HOOK.lock().unwrap();
        if hook
            .as_ref()
            .is_some_and(|hook| hook.path == path && Some(hook.id) == id)
        {
            hook.take()
        } else {
            None
        }
    };
    if let Some(hook) = hook {
        let _ = hook.reached.send(());
        let _ = hook.release.recv();
    }
}
