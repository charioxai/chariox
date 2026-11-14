//! Native invalidations only: no timers scan files or read their contents.
use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use super::{Machine, StateGuard};

pub(crate) const MAX_FILE_WATCH_PATHS: usize = 64;
const COALESCE: Duration = Duration::from_millis(20);

pub(crate) struct FileWatch {
    watcher: RecommendedWatcher,
    /// The paths the client asked for. A link in one of them is followed afresh at every
    /// invalidation, because kernel reads and listings follow it afresh too.
    requested: BTreeSet<PathBuf>,
    /// What an event is checked against: the requested paths and where they lead now.
    interests: Arc<Mutex<BTreeSet<PathBuf>>>,
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
        let requested = paths
            .iter()
            .map(|path| {
                let path = Machine::absolute(path)?;
                guard.check(&path)?;
                // Registration refuses a path that does not exist, just as the file read does.
                path.canonicalize()
                    .map_err(|e| format!("cannot watch {}: {e}", path.display()))?;
                Ok(path)
            })
            .collect::<Result<BTreeSet<_>, String>>()?;
        let interests = Arc::new(Mutex::new(BTreeSet::<PathBuf>::new()));
        let callback_interests = Arc::clone(&interests);
        let (tx, changed) = mpsc::channel(1);
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let relevant = match event {
                Ok(event) if matches!(event.kind, EventKind::Access(_)) => false,
                Ok(event) => {
                    event.need_rescan()
                        || event.paths.iter().any(|path| {
                            let interests = callback_interests.lock().expect("watch interests");
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
            requested,
            interests,
            watched: BTreeMap::new(),
            guard,
            changed,
            pending: false,
        };
        watch.rearm()?;
        Ok(watch)
    }

    /// Resolves every requested path again and watches the directories that can change what
    /// it reads: its target, and the nearest existing parent of the path and of the target. The
    /// parents stay watched when an editor replaces a file, a link is repointed or a directory
    /// is removed and recreated.
    fn rearm(&mut self) -> Result<(), String> {
        let mut interests = self.requested.clone();
        let mut directories = BTreeSet::new();
        for path in &self.requested {
            self.guard.check(path)?;
            directories.extend(nearest_parent_directory(path));
            // A path that is gone, or a link whose target is gone, keeps its parent watched.
            if let Ok(target) = path.canonicalize() {
                self.guard.check(&target)?;
                if target.is_dir() {
                    directories.insert(target.clone());
                }
                directories.extend(nearest_parent_directory(&target));
                interests.insert(target);
            }
        }
        *self.interests.lock().expect("watch interests") = interests;
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

fn nearest_parent_directory(path: &std::path::Path) -> Option<PathBuf> {
    path.ancestors()
        .skip(1)
        .find(|parent| parent.is_dir())
        .map(std::path::Path::to_path_buf)
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

    #[tokio::test]
    async fn native_watch_follows_a_link_repointed_after_registration() {
        let root = std::env::temp_dir().join(format!("files-watch-{}", rand::random::<u64>()));
        let (a, b, links) = (
            root.join("releases/a"),
            root.join("releases/b"),
            root.join("links"),
        );
        for dir in [&a, &b, &links] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let current = links.join("current");
        std::os::unix::fs::symlink(&a, &current).unwrap();
        let mut watch =
            FileWatch::new(&[current.display().to_string()], StateGuard::default()).unwrap();
        let next = links.join("current.next");
        std::os::unix::fs::symlink(&b, &next).unwrap();
        std::fs::rename(&next, &current).unwrap();
        tokio::time::timeout(Duration::from_millis(500), watch.changed())
            .await
            .expect("repointing the link invalidates the listing")
            .unwrap();
        std::fs::write(b.join("new"), "after").unwrap();
        tokio::time::timeout(Duration::from_millis(500), watch.changed())
            .await
            .expect("a change in the new target invalidates the listing")
            .unwrap();
        std::fs::write(a.join("old"), "ignored").unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), watch.changed())
                .await
                .is_err(),
            "the previous target is no longer watched"
        );
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
