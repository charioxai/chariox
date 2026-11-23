//! The file that names the running kernel: `kernel-pid/<port>.pid` in the state
//! directory, next to the local auth token file, written by the kernel itself so
//! no launcher has to keep it right. Every start writes it again, a hot reload
//! included (the new program keeps the PID and replaces the file), and a planned
//! stop removes it unless another kernel has replaced it since.

use std::path::{Path, PathBuf};

use crate::config::DaemonConfig;

pub struct KernelPidFile {
    path: PathBuf,
    #[cfg(unix)]
    identity: (u64, u64),
}

impl KernelPidFile {
    pub fn path_for(port: u16) -> PathBuf {
        DaemonConfig::default_kernel_pid_path(port)
    }

    /// Names this process as the kernel listening on `port`.
    pub fn write(port: u16) -> std::io::Result<Self> {
        Self::write_at(Self::path_for(port))
    }

    fn write_at(path: PathBuf) -> std::io::Result<Self> {
        crate::config::write_private_file(
            &path,
            format!("{}\n", std::process::id()).as_bytes(),
        )?;
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            let metadata = std::fs::symlink_metadata(&path)?;
            (metadata.dev(), metadata.ino())
        };
        Ok(Self {
            path,
            #[cfg(unix)]
            identity,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for KernelPidFile {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let Ok(metadata) = std::fs::symlink_metadata(&self.path) else {
                return;
            };
            if (metadata.dev(), metadata.ino()) != self.identity {
                return;
            }
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_names_this_process_and_a_stop_removes_it() {
        let dir = std::env::temp_dir().join(format!("kernel-pid-file-{}", std::process::id()));
        let path = dir.join("kernel-pid/1.pid");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "1\n").unwrap();
        let file = KernelPidFile::write_at(path.clone()).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().trim(),
            std::process::id().to_string()
        );
        drop(file);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stop_leaves_the_file_a_newer_kernel_wrote() {
        let dir = std::env::temp_dir().join(format!("kernel-pid-newer-{}", std::process::id()));
        let path = dir.join("2.pid");
        std::fs::create_dir_all(&dir).unwrap();
        let old = KernelPidFile::write_at(path.clone()).unwrap();
        let newer = KernelPidFile::write_at(path.clone()).unwrap();
        drop(old);
        assert!(path.exists(), "the newer kernel's file stays");
        drop(newer);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
