//! Allocated growth of a command's open output files, by receiving volume.
use super::VolumeWrites;
#[cfg(target_os = "linux")]
use std::collections::{BTreeMap, BTreeSet};
#[cfg(target_os = "linux")]
use std::path::PathBuf;

#[derive(Default)]
pub(in crate::runtime::process_resources) struct WriteGrowth {
    #[cfg(target_os = "linux")]
    files: BTreeMap<(u64, u64), FileGrowth>,
    /// A receiving inode and a directory on its volume, retained between files.
    #[cfg(target_os = "linux")]
    last: Option<(u64, u64, PathBuf)>,
}

#[cfg(target_os = "linux")]
#[derive(Clone, PartialEq, Eq)]
struct Descriptor {
    position: Option<u64>,
    flags: u32,
}

#[cfg(target_os = "linux")]
struct FileSnapshot {
    allocated: u64,
    length: u64,
    modified: (i64, i64),
    block_size: u64,
    descriptors: BTreeMap<(u32, u32), Descriptor>,
    path: PathBuf,
    stream: bool,
}

#[cfg(target_os = "linux")]
struct FileGrowth {
    allocated: u64,
    at: u64,
    written: u64,
    position_written: u64,
    length: u64,
    modified: (i64, i64),
    descriptors: BTreeMap<(u32, u32), Descriptor>,
}

#[cfg(target_os = "linux")]
impl FileSnapshot {
    fn position_added(&self, before: Option<&FileGrowth>, at: u64) -> Option<u64> {
        if self.descriptors.values().any(|fd| fd.position.is_none()) {
            return None;
        }
        let Some(before) = before else {
            return Some(0);
        };
        if at < before.at {
            return None;
        }
        // Aliases need not share an open description to prove that every
        // observed cursor and the file itself stayed idle.
        if self.descriptors == before.descriptors
            && self.modified == before.modified
            && self.length == before.length
        {
            return Some(0);
        }
        if self.descriptors.len() != 1 || before.descriptors.len() != 1 {
            return None;
        }
        let (key, descriptor) = self.descriptors.iter().next()?;
        let old = before.descriptors.get(key)?;
        if old.flags != descriptor.flags
            || descriptor.flags & libc::O_ACCMODE as u32 != libc::O_WRONLY as u32
            || self.modified == before.modified
        {
            return None;
        }
        let position = descriptor.position?;
        let added = if descriptor.flags & libc::O_APPEND as u32 != 0 {
            // A zero initial cursor must not credit an existing file prefix.
            if position < before.length || position > self.length || self.length < before.length {
                return None;
            }
            position - before.length
        } else {
            position.checked_sub(old.position?)?
        };
        let allocated = self.allocated.saturating_sub(before.allocated);
        // Neither sparse extension nor a seek proves written bytes. Excess
        // allocation can also be an older excluded write becoming visible.
        // Allow only the filesystem's allocation rounding around new progress.
        (added > 0 && allocated >= added && allocated - added < self.block_size).then_some(added)
    }
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
        let mut snapshots = BTreeMap::<(u64, u64), FileSnapshot>::new();
        // Partial attribution cannot justify a stop; bound each monitor tick.
        let mut budget = 16_384usize;
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
                let Some(number) = fd
                    .file_name()
                    .to_str()
                    .and_then(|name| name.parse::<u32>().ok())
                else {
                    continue;
                };
                let Ok(info) = std::fs::read_to_string(base.join("fdinfo").join(fd.file_name()))
                else {
                    continue;
                };
                let Some(flags) = info
                    .lines()
                    .find_map(|line| line.strip_prefix("flags:"))
                    .and_then(|flags| u32::from_str_radix(flags.trim(), 8).ok())
                else {
                    continue;
                };
                if flags & libc::O_ACCMODE as u32 == libc::O_RDONLY as u32 {
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
                let position = info
                    .lines()
                    .find_map(|line| line.strip_prefix("pos:"))
                    .and_then(|value| value.trim().parse::<u64>().ok())
                    .filter(|_| {
                        info.lines()
                            .find_map(|line| line.strip_prefix("ino:"))
                            .and_then(|value| value.trim().parse::<u64>().ok())
                            == Some(key.1)
                    });
                let snapshot = snapshots.entry(key).or_insert_with(|| FileSnapshot {
                    allocated: 0,
                    length: 0,
                    modified: (0, 0),
                    block_size: 1,
                    descriptors: BTreeMap::new(),
                    path,
                    stream: false,
                });
                snapshot.allocated = meta.blocks().saturating_mul(512);
                snapshot.length = meta.len();
                snapshot.modified = (meta.mtime(), meta.mtime_nsec());
                snapshot.block_size = meta.blksize().max(1);
                snapshot.stream |= number <= 2;
                snapshot.descriptors.insert(
                    (*pid, number),
                    Descriptor {
                        position,
                        flags: flags & (libc::O_ACCMODE | libc::O_APPEND) as u32,
                    },
                );
            }
        }
        let streams = snapshots
            .iter()
            .filter_map(|(key, value)| value.stream.then_some(*key))
            .collect::<BTreeSet<_>>();
        if self
            .last
            .as_ref()
            .is_some_and(|(id, inode, _)| streams.contains(&(*id, *inode)))
        {
            // An alias of a standard stream is not a separate receiving file.
            self.last = None;
        }
        let mut files = BTreeMap::new();
        let mut volumes = BTreeMap::<u64, VolumeWrites>::new();
        let mut outputs = BTreeMap::<u64, (u64, PathBuf)>::new();
        for (key, snapshot) in snapshots {
            let before = self.files.get(&key);
            let (written, rate) = match before {
                Some(before) if at > before.at => {
                    let added = snapshot.allocated.saturating_sub(before.allocated);
                    (
                        before.written.saturating_add(added),
                        added.saturating_mul(1000) / (at - before.at),
                    )
                }
                Some(before) => (before.written, 0),
                None => (0, 0),
            };
            let position_added = snapshot.position_added(before, at);
            let position_written = match before {
                Some(before) if at > before.at => before
                    .position_written
                    .saturating_add(position_added.unwrap_or(0)),
                Some(before) => before.position_written,
                None => 0,
            };
            let volume = volumes.entry(key.0).or_insert_with(|| VolumeWrites {
                id: key.0,
                path: snapshot.path.clone(),
                written: 0,
                rate: 0,
                files: Default::default(),
                open: Default::default(),
                written_by_inode: Default::default(),
                position_written_by_inode: Default::default(),
                receiving: false,
            });
            volume.open.insert(key.1);
            volume.written_by_inode.insert(key.1, written);
            if position_added.is_some() {
                volume
                    .position_written_by_inode
                    .insert(key.1, position_written);
            }
            if !snapshot.stream {
                volume.receiving = true;
                outputs
                    .entry(key.0)
                    .or_insert_with(|| (key.1, snapshot.path.clone()));
            }
            if rate > 0 {
                volume.files.insert(key.1);
            }
            volume.written = volume.written.saturating_add(written);
            volume.rate = volume.rate.saturating_add(rate);
            let (allocated, at, length, modified, descriptors) =
                before.filter(|before| at <= before.at).map_or(
                    (
                        snapshot.allocated,
                        at,
                        snapshot.length,
                        snapshot.modified,
                        snapshot.descriptors,
                    ),
                    |before| {
                        (
                            before.allocated,
                            before.at,
                            before.length,
                            before.modified,
                            before.descriptors.clone(),
                        )
                    },
                );
            files.insert(
                key,
                FileGrowth {
                    allocated,
                    at,
                    written,
                    position_written,
                    length,
                    modified,
                    descriptors,
                },
            );
        }
        self.files = files;
        self.remember(&mut volumes, &outputs);
        volumes.into_values().collect()
    }

    /// A many-file copy is between two files at most samples. Retain its
    /// receiving directory while keeping standard-stream aliases separate.
    #[cfg(target_os = "linux")]
    fn remember(
        &mut self,
        volumes: &mut BTreeMap<u64, VolumeWrites>,
        outputs: &BTreeMap<u64, (u64, PathBuf)>,
    ) {
        match outputs.iter().next() {
            None => {
                let Some((id, _, dir)) = self.last.as_ref() else {
                    return;
                };
                let volume = volumes.entry(*id).or_insert_with(|| VolumeWrites {
                    id: *id,
                    path: dir.clone(),
                    written: 0,
                    rate: 0,
                    files: Default::default(),
                    open: Default::default(),
                    written_by_inode: Default::default(),
                    position_written_by_inode: Default::default(),
                    receiving: false,
                });
                volume.path = dir.clone();
                volume.receiving = true;
            }
            Some((id, (inode, output))) if outputs.len() == 1 => {
                let dir = std::fs::canonicalize(output)
                    .ok()
                    .and_then(|file| file.parent().map(PathBuf::from));
                if let Some(dir) = dir {
                    self.last = Some((*id, *inode, dir));
                }
            }
            Some(_) => self.last = None,
        }
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

    #[test]
    fn a_closed_output_leaves_its_volume_known_while_the_command_lives() {
        use std::io::{BufRead, BufReader};
        let dir = std::env::temp_dir().join(format!("volume-last-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("copy-0");
        // A writer of its own: the test process may hold other files open.
        let mut child = std::process::Command::new("python3")
            .args([
                "-c",
                "import sys\nf=open(sys.argv[1],'wb'); f.write(bytes(4096)); f.flush(); print('open',flush=True)\nsys.stdin.readline(); f.close(); print('closed',flush=True)\nsys.stdin.readline()",
            ])
            .arg(&path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            // A log on the same volume through a standard stream.
            .stderr(std::fs::File::create(dir.join("err.log")).unwrap())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let mut growth = WriteGrowth::default();
        let pids = [child.id()];
        let open = growth.sample(&pids, 1000);
        assert_eq!(open.len(), 1, "{open:?}");
        assert_eq!(open[0].id, meta.dev());
        assert!(
            open[0].open.contains(&meta.ino()),
            "an open output is not held"
        );
        input.write_all(b"close\n").unwrap();
        line.clear();
        output.read_line(&mut line).unwrap();
        let between = growth.sample(&pids, 2500);
        assert_eq!(
            between.len(),
            1,
            "the volume was forgotten between two files"
        );
        assert_eq!(between[0].id, meta.dev());
        assert_eq!(between[0].path, std::fs::canonicalize(&dir).unwrap());
        assert!(between[0].receiving && between[0].files.is_empty());
        assert!(
            !between[0].open.contains(&meta.ino()),
            "a closed output is held"
        );
        assert!(
            growth
                .sample(&pids, 60_000)
                .iter()
                .any(|volume| volume.receiving && volume.id == meta.dev()),
            "the volume was forgotten while the command still lives"
        );
        drop(input);
        child.wait().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
