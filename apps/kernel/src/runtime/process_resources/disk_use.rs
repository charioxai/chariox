//! What a process keeps on disk besides what it wrote: files that were
//! deleted while it still holds them open. They take their space until the
//! holder lets go, and no directory listing shows them.
use std::collections::BTreeSet;

mod writes;
pub(super) use writes::WriteGrowth;

/// Space a command actually added on one receiving volume.
#[derive(Clone, Debug)]
pub(super) struct VolumeWrites {
    pub id: u64,
    pub path: std::path::PathBuf,
    pub written: u64,
    pub rate: u64,
    /// Growing writable inodes; shared writers cannot be attributed safely.
    pub files: BTreeSet<u64>,
}

/// Below this written, a process is not looked at: checking its descriptors
/// costs more than the space it could hold.
pub(super) const LOOK_AT_WRITTEN: u64 = 64 * super::MIB;

/// Bytes of deleted regular files `pid` holds open, each file once.
#[cfg(target_os = "linux")]
pub(super) fn deleted_open_bytes(pid: u32) -> u64 {
    use std::os::unix::fs::MetadataExt;
    let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
        return 0;
    };
    let mut seen = BTreeSet::new();
    let mut bytes = 0;
    for fd in fds.flatten() {
        let deleted = std::fs::read_link(fd.path())
            .is_ok_and(|target| target.to_string_lossy().ends_with(" (deleted)"));
        if !deleted {
            continue;
        }
        // Following the descriptor reaches the deleted file itself.
        let Ok(meta) = std::fs::metadata(fd.path()) else {
            continue;
        };
        if meta.is_file() && seen.insert((meta.dev(), meta.ino())) {
            bytes += meta.blocks() * 512;
        }
    }
    bytes
}

#[cfg(not(target_os = "linux"))]
pub(super) fn deleted_open_bytes(_pid: u32) -> u64 {
    0
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn a_deleted_file_a_process_holds_open_counts_against_its_holder() {
        let path = std::env::temp_dir().join(format!("disk-use-{}", std::process::id()));
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(&vec![7u8; 3 * 1024 * 1024]).unwrap();
        file.sync_all().unwrap();
        assert_eq!(deleted_open_bytes(std::process::id()), 0);
        std::fs::remove_file(&path).unwrap();
        let held = deleted_open_bytes(std::process::id());
        assert!(held >= 3 * 1024 * 1024, "held {held}");
        drop(file);
        assert_eq!(deleted_open_bytes(std::process::id()), 0);
    }
}
