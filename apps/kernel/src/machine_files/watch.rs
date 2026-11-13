//! Native invalidations only: no timers scan files or read their contents.
use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use super::{Machine, StateGuard};

pub(crate) const MAX_FILE_WATCH_PATHS: usize = 64;
const COALESCE: Duration = Duration::from_millis(20);

pub(crate) struct FileWatch {
    watcher: RecommendedWatcher,
    paths: BTreeSet<PathBuf>,
    watched: BTreeMap<PathBuf, (u64, u64)>,
    guard: StateGuard,
    changed: mpsc::Receiver<()>,
    pending: bool,
}

impl FileWatch {
    pub(crate) fn new(paths: &[String], guard: StateGuard) -> Result<Self, String> {
        if paths.is_empty() || paths.len() > MAX_FILE_WATCH_PATHS {
            return Err(format!(
                "watch between 1 and {MAX_FILE_WATCH_PATHS} paths per client"
            ));
        }
        let paths = paths
            .iter()
            .map(|path| {
                let path = Machine::absolute(path)?;
                guard.check(&path)?;
                // Resolve links at registration, just as the file read does. The parent remains
                // watched when an editor replaces a file or a directory is removed and recreated.
                let real = path
                    .canonicalize()
                    .map_err(|e| format!("cannot watch {}: {e}", path.display()))?;
                guard.check(&real)?;
                Ok(real)
            })
            .collect::<Result<BTreeSet<_>, String>>()?;
        let interests = paths.clone();
        let (tx, changed) = mpsc::channel(1);
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let relevant = match event {
                Ok(event) if matches!(event.kind, EventKind::Access(_)) => false,
                Ok(event) => {
                    event.need_rescan()
                        || event.paths.iter().any(|path| {
                            interests.iter().any(|interest| {
                                path == interest
                                    || interest.starts_with(path)
                                    || path.parent() == Some(interest.as_path())
                            })
                        })
                }
                // Overflow/backend errors require a resync of every interest, never a partial view.
                Err(_) => true,
            };
            if relevant {
                let _ = tx.try_send(());
            }
        })
        .map_err(|e| format!("native filesystem notifications are unavailable: {e}"))?;
        let mut watch = Self {
            watcher,
            paths,
            watched: BTreeMap::new(),
            guard,
            changed,
            pending: false,
        };
        watch.rearm()?;
        Ok(watch)
    }

    fn rearm(&mut self) -> Result<(), String> {
        let mut directories = BTreeSet::new();
        for path in &self.paths {
            self.guard.check(path)?;
            if path.is_dir() {
                directories.insert(path.clone());
            }
            if let Some(parent) = path.ancestors().skip(1).find(|parent| parent.is_dir()) {
                directories.insert(parent.to_path_buf());
            }
        }
        self.watched.retain(|path, _| {
            if directories.contains(path) {
                true
            } else {
                let _ = self.watcher.unwatch(path);
                false
            }
        });
        for directory in directories {
            self.guard.check(&directory)?;
            let metadata = std::fs::metadata(&directory).map_err(|e| e.to_string())?;
            let identity = (metadata.dev(), metadata.ino());
            if self.watched.get(&directory) == Some(&identity) {
                continue;
            }
            self.watcher
                .watch(&directory, RecursiveMode::NonRecursive)
                .map_err(|e| format!("cannot watch {}: {e}", directory.display()))?;
            self.watched.insert(directory, identity);
        }
        Ok(())
    }

    pub(crate) async fn changed(&mut self) -> Result<(), String> {
        if !self.pending {
            self.changed
                .recv()
                .await
                .ok_or("native filesystem watch ended")?;
            // Retain this bit if a lease renewal wins the worker's select while
            // coalescing, so cancellation cannot consume and lose an event.
            self.pending = true;
        }
        // A fixed window bounds latency even when writes never stop. A one-slot inbox
        // bounds memory and turns an arbitrary burst into one complete invalidation.
        tokio::time::sleep(COALESCE).await;
        while self.changed.try_recv().is_ok() {}
        self.pending = false;
        self.rearm()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn native_watch_coalesces_atomic_replacement_and_rearms_a_directory() {
        let root = std::env::temp_dir().join(format!("files-watch-{}", rand::random::<u64>()));
        let dir = root.join("open");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("note");
        std::fs::write(&file, "before").unwrap();
        let mut watch = FileWatch::new(
            &[dir.display().to_string(), file.display().to_string()],
            StateGuard::default(),
        )
        .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(60), watch.changed())
                .await
                .is_err()
        );
        std::fs::write(dir.join("swap"), "after").unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(5), watch.changed())
                .await
                .is_err()
        );
        std::fs::rename(dir.join("swap"), &file).unwrap();
        tokio::time::timeout(Duration::from_millis(200), watch.changed())
            .await
            .unwrap()
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(60), watch.changed())
                .await
                .is_err()
        );
        std::fs::remove_dir_all(&dir).unwrap();
        tokio::time::timeout(Duration::from_millis(200), watch.changed())
            .await
            .unwrap()
            .unwrap();
        std::fs::create_dir(&dir).unwrap();
        tokio::time::timeout(Duration::from_millis(200), watch.changed())
            .await
            .unwrap()
            .unwrap();
        std::fs::write(&file, "recreated").unwrap();
        tokio::time::timeout(Duration::from_millis(200), watch.changed())
            .await
            .unwrap()
            .unwrap();
        drop(watch);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn native_watch_refuses_unbounded_and_protected_interests() {
        assert!(FileWatch::new(&vec!["/".into(); 65], StateGuard::default()).is_err());
        assert!(FileWatch::new(&[], StateGuard::default()).is_err());
        let guard = StateGuard::new(Some("/var/tmp/files-watch-vault"));
        assert!(
            FileWatch::new(&["/var/tmp/files-watch-vault".into()], guard)
                .err()
                .unwrap()
                .contains("state or Vault")
        );
    }
}
