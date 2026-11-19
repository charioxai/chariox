use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Set to keep test directories for debugging; isolated tests forward it to their child.
pub(crate) const KEEP_VARIABLE: &str = "CHARIOX_KEEP_TEST_TEMP";

/// Removes a test's directory, or keeps it and says where when `KEEP_VARIABLE` is set.
/// Returns whether it was removed.
pub(crate) fn remove_or_keep(path: &Path) -> bool {
    if std::env::var_os(KEEP_VARIABLE).is_none() {
        let _ = std::fs::remove_dir_all(path);
        return true;
    }
    // Written past the test harness's output capture, which hides passing tests.
    let _ = writeln!(std::io::stderr(), "kept test directory {}", path.display());
    false
}

/// A directory one test owns. It is removed when the guard drops, whether the
/// test passes, fails or panics; `CHARIOX_KEEP_TEST_TEMP=1` keeps it for debugging.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TestTempDir {
    path: PathBuf,
}

impl TestTempDir {
    /// Creates `<tmp>/chariox-tests/<label>-<pid>-<n>`.
    pub(crate) fn new(label: &str) -> Self {
        // macOS reaches its temporary root through /var -> /private/var, and
        // private storage rejects symlink ancestors.
        let root = std::fs::canonicalize(std::env::temp_dir())
            .expect("test temporary root must exist")
            .join("chariox-tests");
        let path = root.join(format!(
            "{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed) + 1
        ));
        std::fs::create_dir_all(&path).expect("test temporary directory should be created");
        Self { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl std::ops::Deref for TestTempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for TestTempDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestTempDir {
    fn drop(&mut self) {
        remove_or_keep(&self.path);
    }
}

#[cfg(test)]
/// A file path inside a directory of its own, so the file and the journal files
/// a database writes beside it go together.
#[derive(Debug)]
pub(crate) struct TestTempFile {
    path: PathBuf,
    // Declared last: it is the owner of `path`.
    _dir: TestTempDir,
}

#[cfg(test)]
impl TestTempFile {
    pub(crate) fn new(label: &str, file_name: &str) -> Self {
        let dir = TestTempDir::new(label);
        Self {
            path: dir.join(file_name),
            _dir: dir,
        }
    }
}

#[cfg(test)]
impl std::ops::Deref for TestTempFile {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
impl AsRef<Path> for TestTempFile {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
/// A path the code under test names itself, under the system temporary
/// directory, so the test cannot place it inside a `TestTempDir`. Removed on
/// drop even when that code already removed it.
pub(crate) struct TestTempPath {
    path: PathBuf,
    remove_empty_parent: bool,
}

#[cfg(test)]
impl TestTempPath {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            remove_empty_parent: false,
        }
    }

    /// For a path inside a directory the code under test shares between runs:
    /// that directory goes too once nothing else is left in it.
    pub(crate) fn in_shared_directory(path: PathBuf) -> Self {
        Self {
            path,
            remove_empty_parent: true,
        }
    }
}

#[cfg(test)]
impl Drop for TestTempPath {
    fn drop(&mut self) {
        if !remove_or_keep(&self.path) {
            return;
        }
        if let Some(parent) = self.path.parent().filter(|_| self.remove_empty_parent) {
            let _ = std::fs::remove_dir(parent);
        }
    }
}
