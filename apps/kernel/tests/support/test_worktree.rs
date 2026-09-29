#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_WORKTREE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A real, disposable working directory for integration-test sessions.
///
/// Provider launch preflight rejects working directories that do not exist, so
/// sessions that launch a provider need a real worktree. This mirrors the
/// kernel's in-crate `test_support::TestWorktree`, which integration tests
/// cannot reach.
pub struct TestWorktree {
    path: PathBuf,
}

impl TestWorktree {
    pub fn new(label: &str) -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("test clock should follow the epoch")
            .as_millis();
        let sequence = TEST_WORKTREE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "chariox-test-worktree-{label}-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("test worktree should exist");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn path_string(&self) -> String {
        self.path.display().to_string()
    }
}

impl Drop for TestWorktree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
