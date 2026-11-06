//! Allocated growth of a command's open output files, by receiving volume.
use super::VolumeWrites;
#[cfg(target_os = "linux")]
use std::collections::BTreeMap;
#[cfg(target_os = "linux")]
use std::path::PathBuf;

#[derive(Default)]
pub(in crate::runtime::process_resources) struct WriteGrowth {
    #[cfg(target_os = "linux")]
    files: BTreeMap<(u64, u64), FileGrowth>,
}

#[cfg(target_os = "linux")]
struct FileGrowth {
    allocated: u64,
    at: u64,
    written: u64,
}

impl WriteGrowth {
    /// Existing bytes are a baseline, never evidence that this command added
    /// them. Closed descriptors leave the cache on the next sample.
    #[cfg(target_os = "linux")]
    pub(in crate::runtime::process_resources) fn sample(
        &mut self,
        pids: &[u32],
        at: u64,
    ) -> Vec<VolumeWrites> {
        use std::os::unix::fs::MetadataExt;
        let mut files = BTreeMap::new();
        // Partial attribution cannot justify a stop; bound each monitor tick.
        let mut budget = 16_384usize;
        let mut volumes = BTreeMap::<u64, VolumeWrites>::new();
        for pid in pids {
            let base = PathBuf::from(format!("/proc/{pid}"));
            let Ok(fds) = std::fs::read_dir(base.join("fd")) else {
                continue;
            };
            for fd in fds.flatten() {
                let Some(left) = budget.checked_sub(1) else {
                    self.files.clear();
                    return Vec::new();
                };
                budget = left;
                let Ok(info) = std::fs::read_to_string(base.join("fdinfo").join(fd.file_name()))
                else {
                    continue;
                };
                let writable = info
                    .lines()
                    .find_map(|line| line.strip_prefix("flags:"))
                    .and_then(|flags| u32::from_str_radix(flags.trim(), 8).ok())
                    .is_some_and(|flags| flags & libc::O_ACCMODE as u32 != libc::O_RDONLY as u32);
                if !writable {
                    continue;
                }
                let path = fd.path();
                let Ok(meta) = std::fs::metadata(&path) else {
                    continue;
                };
                if !meta.is_file() {
                    continue;
                }
                let key = (meta.dev(), meta.ino());
                if files.contains_key(&key) {
                    continue;
                }
                let allocated = meta.blocks().saturating_mul(512);
                let before = self.files.get(&key);
                // Only this interval's growth can justify stopping a writer.
                // Decay would retain another volume's earlier write activity.
                let (written, rate) = match before {
                    Some(before) if at > before.at => {
                        let added = allocated.saturating_sub(before.allocated);
                        (
                            before.written.saturating_add(added),
                            added.saturating_mul(1000) / (at - before.at),
                        )
                    }
                    Some(before) => (before.written, 0),
                    None => (0, 0),
                };
                let (allocated, at) = before
                    .filter(|before| at <= before.at)
                    .map_or((allocated, at), |before| (before.allocated, before.at));
                files.insert(
                    key,
                    FileGrowth {
                        allocated,
                        at,
                        written,
                    },
                );
                let volume = volumes.entry(key.0).or_insert_with(|| VolumeWrites {
                    id: key.0,
                    path,
                    written: 0,
                    rate: 0,
                    files: Default::default(),
                });
                if rate > 0 {
                    volume.files.insert(key.1);
                }
                volume.written = volume.written.saturating_add(written);
                volume.rate = volume.rate.saturating_add(rate);
            }
        }
        self.files = files;
        volumes.into_values().collect()
    }

    #[cfg(not(target_os = "linux"))]
    pub(in crate::runtime::process_resources) fn sample(
        &mut self,
        _pids: &[u32],
        _at: u64,
    ) -> Vec<VolumeWrites> {
        Vec::new()
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::io::{Seek, Write};
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn growth_uses_the_open_files_volume_and_ignores_existing_or_overwritten_bytes() {
        let path = std::env::temp_dir().join(format!("volume-growth-{}", std::process::id()));
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(&vec![7u8; 2 * 1024 * 1024]).unwrap();
        file.sync_all().unwrap();
        let meta = file.metadata().unwrap();
        let id = meta.dev();
        let inode = meta.ino();
        let mut growth = WriteGrowth::default();
        let pids = [std::process::id()];
        let first = growth.sample(&pids, 1000);
        assert_eq!(first.iter().map(|volume| volume.written).sum::<u64>(), 0);
        file.write_all(&vec![8u8; 2 * 1024 * 1024]).unwrap();
        file.sync_all().unwrap();
        let cached = growth.sample(&pids, 1000);
        assert_eq!(cached.iter().map(|volume| volume.written).sum::<u64>(), 0);
        let added = growth.sample(&pids, 2000);
        let volume = added.iter().find(|volume| volume.id == id).unwrap();
        assert!(volume.written >= 2 * 1024 * 1024);
        let written = volume.written;
        file.rewind().unwrap();
        file.write_all(&vec![9u8; 1024 * 1024]).unwrap();
        file.sync_all().unwrap();
        let overwrite = growth.sample(&pids, 3000);
        assert_eq!(
            overwrite
                .iter()
                .find(|volume| volume.id == id)
                .unwrap()
                .written,
            written
        );
        let idle_volume = overwrite.iter().find(|volume| volume.id == id).unwrap();
        assert_eq!(idle_volume.rate, 0, "past growth is not a current writer");
        assert!(!idle_volume.files.contains(&inode));
        std::fs::remove_file(&path).unwrap();
        file.seek(std::io::SeekFrom::End(0)).unwrap();
        file.write_all(&vec![1u8; 2 * 1024 * 1024]).unwrap();
        file.sync_all().unwrap();
        let deleted = growth.sample(&pids, 4000);
        assert!(
            deleted
                .iter()
                .find(|volume| volume.id == id)
                .unwrap()
                .written
                > written
        );
        drop(file);
        growth.sample(&pids, 5000);
        assert!(!growth.files.contains_key(&(id, inode)));
    }
}
