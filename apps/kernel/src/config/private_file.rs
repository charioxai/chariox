use std::fs;
use std::io;
use std::path::Path;

#[cfg(not(windows))]
use std::fs::OpenOptions;
#[cfg(not(windows))]
use std::io::Write;
#[cfg(not(windows))]
use std::path::PathBuf;

#[cfg(not(windows))]
use rand::RngCore;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub(crate) fn write_private_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "private file has no parent"))?;
    fs::create_dir_all(parent)?;
    write_private_file_platform(path, parent, contents)
}

#[cfg(windows)]
fn write_private_file_platform(path: &Path, _parent: &Path, contents: &[u8]) -> io::Result<()> {
    // Preserve the prior replace behavior on Windows. Managed kernels run on Linux;
    // Windows does not support Unix rename-overwrite or directory fsync semantics.
    fs::write(path, contents)
}

#[cfg(not(windows))]
fn write_private_file_platform(path: &Path, parent: &Path, contents: &[u8]) -> io::Result<()> {
    let temporary = temporary_path(path);
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
fn temporary_path(path: &Path) -> PathBuf {
    let mut suffix = [0_u8; 8];
    rand::thread_rng().fill_bytes(&mut suffix);
    let suffix = suffix
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("private");
    path.with_file_name(format!(".{name}.{suffix}.tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_file_replaces_atomically_with_private_permissions() {
        let root = std::env::temp_dir().join(format!(
            "chariox-private-file-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let path = root.join("nested").join("secret.json");
        write_private_file(&path, b"first").expect("first private write");
        write_private_file(&path, b"second").expect("replacement private write");
        assert_eq!(fs::read(&path).expect("read private file"), b"second");
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&path)
                .expect("private metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let leftovers = fs::read_dir(path.parent().expect("private parent"))
            .expect("list private parent")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
        let _ = fs::remove_dir_all(root);
    }
}
