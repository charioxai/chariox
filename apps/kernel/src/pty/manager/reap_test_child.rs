//! Keep a real child's exit pending to exercise cleanup retries.
use super::*;
use std::sync::atomic::AtomicBool;

#[derive(Debug)]
struct DeferredReapChild {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    ready: Arc<AtomicBool>,
}

impl portable_pty::ChildKiller for DeferredReapChild {
    fn kill(&mut self) -> std::io::Result<()> {
        // The production PTY group kill already terminates the real child.
        #[cfg(unix)]
        {
            Ok(())
        }
        #[cfg(not(unix))]
        {
            self.child.kill()
        }
    }

    fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
        self.child.clone_killer()
    }
}

impl portable_pty::Child for DeferredReapChild {
    fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
        if self.ready.load(Ordering::Acquire) {
            self.child.try_wait()
        } else {
            Ok(None)
        }
    }

    fn wait(&mut self) -> std::io::Result<portable_pty::ExitStatus> {
        Err(std::io::Error::other(
            "process removal attempted a blocking reap",
        ))
    }

    fn process_id(&self) -> Option<u32> {
        self.child.process_id()
    }

    #[cfg(windows)]
    fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        self.child.as_raw_handle()
    }
}

impl PtyManager {
    pub(crate) fn defer_reap_for_test(&mut self, provider_run_id: &str, ready: Arc<AtomicBool>) {
        let key = self.process_key(provider_run_id).unwrap();
        let process = self.processes.remove(&key).unwrap();
        self.processes.insert(
            key,
            PtyProcess {
                child: Box::new(DeferredReapChild {
                    child: process.child,
                    ready,
                }),
                ..process
            },
        );
    }
}
