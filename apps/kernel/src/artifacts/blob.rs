//! MP-08/MP-10/MP-11: publish the exact descriptor-pinned bytes that were hashed.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::{artifact_error, hex_lower};
use crate::error::DaemonError;

pub(super) fn store(root: &Path, source: &Path) -> Result<(String, u64, PathBuf), DaemonError> {
    let source = fs::canonicalize(source)
        .map_err(|error| artifact_error("canonicalize source artifact", error))?;
    let mut input =
        open_regular(&source).map_err(|error| artifact_error("open artifact source", error))?;
    let temporary = root
        .join("blobs")
        .join(format!(".pending-{:032x}", rand::random::<u128>()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options
        .open(&temporary)
        .map_err(|error| artifact_error("create private artifact blob", error))?;
    let _cleanup = TemporaryBlob(temporary.clone());
    let (sha256, size) = copy_and_hash(&mut input, &mut output)
        .map_err(|error| artifact_error("copy artifact blob", error))?;
    output
        .sync_all()
        .map_err(|error| artifact_error("sync artifact blob", error))?;
    let blob = root
        .join("blobs")
        .join(&sha256[..2])
        .join(&sha256[2..4])
        .join(&sha256);
    let parent = blob.parent().expect("blob shard has a parent");
    fs::create_dir_all(parent)
        .map_err(|error| artifact_error("create artifact blob shard", error))?;
    // Publish without replacing a blob another concurrent admission may use.
    match fs::hard_link(&temporary, &blob) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let mut existing = open_regular(&blob)
                .map_err(|error| artifact_error("open existing artifact blob", error))?;
            if existing
                .metadata()
                .map_err(|error| artifact_error("inspect existing artifact blob", error))?
                .len()
                != size
            {
                return Err(artifact_error(
                    "verify existing artifact blob",
                    invalid("artifact blob size mismatch"),
                ));
            }
            // Concurrent publication/cleanup changes the blob's link count and
            // ctime, not its bytes. Its content address supplies the integrity
            // check; mutable source capture still requires stable timestamps.
            let actual = copy_bounded_and_hash(&mut existing, &mut io::sink(), size)
                .map_err(|error| artifact_error("verify existing artifact blob", error))?;
            if actual != (sha256.clone(), size) {
                return Err(artifact_error(
                    "verify existing artifact blob",
                    invalid("artifact blob digest mismatch"),
                ));
            }
        }
        Err(error) => return Err(artifact_error("publish artifact blob", error)),
    }
    Ok((sha256, size, blob))
}

pub(super) fn open_regular(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid("artifact source must be a regular file"));
    }
    Ok(file)
}

fn copy_and_hash(input: &mut File, output: &mut impl Write) -> io::Result<(String, u64)> {
    let before = input.metadata()?;
    let (digest, copied) = copy_bounded_and_hash(input, output, before.len())?;
    let after = input.metadata()?;
    if copied != before.len()
        || before.len() != after.len()
        || before.modified()? != after.modified()?
    {
        return Err(invalid("artifact source changed during capture"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if (before.ctime(), before.ctime_nsec()) != (after.ctime(), after.ctime_nsec()) {
            return Err(invalid("artifact source changed during capture"));
        }
    }
    Ok((digest, copied))
}

fn copy_bounded_and_hash(
    input: &mut File,
    output: &mut impl Write,
    size: u64,
) -> io::Result<(String, u64)> {
    let mut hasher = Sha256::new();
    let mut copied = 0;
    let mut buffer = [0u8; 64 * 1024];
    // A growing source cannot turn an approved finite file into an endless read.
    let mut bounded = input.take(size.saturating_add(1));
    loop {
        let read = bounded.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        hasher.update(&buffer[..read]);
        copied += read as u64;
    }
    Ok((hex_lower(&hasher.finalize()), copied))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

struct TemporaryBlob(PathBuf);
impl Drop for TemporaryBlob {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // MP-08/MP-10/MP-11: remove the publisher's pending link during hashing.
    #[test]
    fn blob_hash_survives_concurrent_pending_link_cleanup() {
        struct PendingCleanup(PathBuf);
        impl Write for PendingCleanup {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                fs::remove_file(&self.0)?;
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let root = TestRoot::new();
        let pending = root.0.join("pending");
        let published = root.0.join("published");
        let bytes = b"approved original";
        fs::write(&pending, bytes).unwrap();
        fs::hard_link(&pending, &published).unwrap();
        let mut opened = open_regular(&published).unwrap();
        let actual = copy_bounded_and_hash(
            &mut opened,
            &mut PendingCleanup(pending.clone()),
            bytes.len() as u64,
        )
        .unwrap();
        assert!(!pending.exists());
        assert_eq!(fs::read(published).unwrap(), bytes);
        assert_eq!(
            actual,
            (hex_lower(&Sha256::digest(bytes)), bytes.len() as u64)
        );
    }

    // MP-08/MP-10/MP-11: source capture still rejects ctime-only mutations.
    #[cfg(unix)]
    #[test]
    fn source_mutation_with_restored_size_and_mtime_is_rejected() {
        struct MutatingOutput {
            source: PathBuf,
            modified: std::time::SystemTime,
        }
        impl Write for MutatingOutput {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                fs::write(&self.source, vec![2; 128 * 1024])?;
                OpenOptions::new()
                    .write(true)
                    .open(&self.source)?
                    .set_modified(self.modified)?;
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let root = TestRoot::new();
        let source = root.0.join("source");
        fs::write(&source, vec![1; 128 * 1024]).unwrap();
        let mut opened = open_regular(&source).unwrap();
        let before = opened.metadata().unwrap();
        let mut output = MutatingOutput {
            source,
            modified: before.modified().unwrap(),
        };
        assert!(copy_and_hash(&mut opened, &mut output).is_err());
        let after = opened.metadata().unwrap();
        assert_eq!(before.len(), after.len());
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    }

    #[test]
    fn copy_hashes_the_opened_inode_after_the_source_path_is_replaced() {
        let root = TestRoot::new();
        let source = root.0.join("source");
        fs::write(&source, b"approved original").unwrap();
        let mut opened = open_regular(&source).unwrap();
        fs::rename(&source, root.0.join("old")).unwrap();
        fs::write(&source, b"replacement").unwrap();
        let mut copied = Vec::new();
        let (digest, size) = copy_and_hash(&mut opened, &mut copied).unwrap();
        assert_eq!(copied, b"approved original");
        assert_eq!(size, copied.len() as u64);
        assert_eq!(digest, hex_lower(&Sha256::digest(&copied)));
    }

    #[test]
    fn changing_or_growing_source_is_rejected_with_a_bounded_copy() {
        struct MutatingOutput {
            source: PathBuf,
            copied: usize,
            grow: bool,
        }
        impl Write for MutatingOutput {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.copied += bytes.len();
                if self.grow {
                    OpenOptions::new()
                        .append(true)
                        .open(&self.source)?
                        .write_all(&[0; 8192])?;
                } else {
                    fs::write(&self.source, b"changed source")?;
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let root = TestRoot::new();
        let source = root.0.join("source");
        for grow in [false, true] {
            fs::write(&source, vec![1; 128 * 1024]).unwrap();
            let mut opened = open_regular(&source).unwrap();
            let mut output = MutatingOutput {
                source: source.clone(),
                copied: 0,
                grow,
            };
            assert!(copy_and_hash(&mut opened, &mut output).is_err());
            assert!(output.copied <= 128 * 1024 + 1);
        }
    }

    #[cfg(unix)]
    #[test]
    fn nonregular_sources_and_symlink_blobs_are_rejected_without_blocking() {
        use std::os::unix::fs::symlink;
        let root = TestRoot::new();
        assert!(open_regular(&root.0).is_err());
        let fifo = root.0.join("fifo");
        let name = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(open_regular(&fifo).is_err());
        let regular = root.0.join("regular");
        fs::write(&regular, b"bytes").unwrap();
        let link = root.0.join("link");
        symlink(regular, &link).unwrap();
        assert!(open_regular(&link).is_err());
    }

    struct TestRoot(PathBuf);
    impl TestRoot {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "chariox-artifact-blob-{:032x}",
                rand::random::<u128>()
            ));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
