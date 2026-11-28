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
    /// The one volume it last sent output to: its id and a directory on it.
    /// A command that writes one short-lived file after another is seen with
    /// a file open in few samples; the volume stands until it is seen sending
    /// output elsewhere.
    #[cfg(target_os = "linux")]
    last: Option<(u64, PathBuf)>,
}

#[cfg(target_os = "linux")]
struct FileGrowth {
    allocated: u64,
    at: u64,
    written: u64,
    position: Option<u64>,
    position_written: u64,
    write_only: bool,
    modified: (i64, i64),
    length: u64,
    append: bool,
    multiple: bool,
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
        let mut files = BTreeMap::<(u64, u64), FileGrowth>::new();
        // Partial attribution cannot justify a stop; bound each monitor tick.
        let mut budget = 16_384usize;
        let mut volumes = BTreeMap::<u64, VolumeWrites>::new();
        // A descriptor (other than a standard stream) per receiving volume.
        let mut outputs = BTreeMap::<u64, PathBuf>::new();
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
                let flags = info
                    .lines()
                    .find_map(|line| line.strip_prefix("flags:"))
                    .and_then(|flags| u32::from_str_radix(flags.trim(), 8).ok());
                let access = flags.map(|flags| flags & libc::O_ACCMODE as u32);
                let append = flags.is_some_and(|flags| flags & libc::O_APPEND as u32 != 0);
                let writable = access.is_some_and(|mode| mode != libc::O_RDONLY as u32);
                let write_only = access == Some(libc::O_WRONLY as u32);
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
                let stream = fd
                    .file_name()
                    .to_str()
                    .and_then(|fd| fd.parse::<u32>().ok())
                    .is_some_and(|fd| fd <= 2);
                if let Some(file) = files.get_mut(&key) {
                    // fdinfo cannot distinguish dup() from an independent open.
                    // One inode must never imply one shared cursor or summed aliases.
                    file.multiple = true;
                    file.position_written = self
                        .files
                        .get(&key)
                        .map_or(0, |before| before.position_written);
                    if let Some(volume) = volumes.get_mut(&key.0) {
                        volume.position_written_by_inode.remove(&key.1);
                        if !stream {
                            volume.receiving = true;
                            outputs.entry(key.0).or_insert_with(|| fd.path());
                        }
                    }
                    continue;
                }
                let allocated = meta.blocks().saturating_mul(512);
                let modified = (meta.mtime(), meta.mtime_nsec());
                let length = meta.len();
                let position = info
                    .lines()
                    .find_map(|line| line.strip_prefix("pos:"))
                    .and_then(|value| value.trim().parse::<u64>().ok())
                    .filter(|_| {
                        // The descriptor may have been reused between fdinfo
                        // and metadata reads. Never pair another inode's offset.
                        info.lines()
                            .find_map(|line| line.strip_prefix("ino:"))
                            .and_then(|value| value.trim().parse::<u64>().ok())
                            .is_some_and(|inode| inode == key.1)
                    });
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
                let position_added = match (position, before) {
                    (Some(_), None) => Some(0),
                    (Some(now), Some(before))
                        if at >= before.at
                            && write_only == before.write_only
                            && append == before.append
                            && !before.multiple =>
                    {
                        match before.position {
                            Some(old) if now == old && modified == before.modified => Some(0),
                            Some(_)
                                if write_only
                                    && append
                                    && modified != before.modified
                                    && length >= before.length
                                    && now >= before.length
                                    && now <= length =>
                            {
                                // An append starts at the prior EOF, even when the
                                // descriptor's first sampled position was zero.
                                Some(now - before.length)
                            }
                            Some(old)
                                if write_only
                                    && !append
                                    && now > old
                                    && modified != before.modified
                                    && now - old <= allocated.saturating_sub(before.allocated) =>
                            {
                                Some(now - old)
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                let position_continuous = position_added.is_some();
                let position_written = match before {
                    Some(before) if at > before.at => before
                        .position_written
                        .saturating_add(position_added.unwrap_or(0)),
                    Some(before) => before.position_written,
                    None => 0,
                };
                let (position, write_only, modified, length, append) = before
                    .filter(|before| at <= before.at)
                    .map_or((position, write_only, modified, length, append), |before| {
                        (
                            before.position,
                            before.write_only,
                            before.modified,
                            before.length,
                            before.append,
                        )
                    });
                let (allocated, at) = before
                    .filter(|before| at <= before.at)
                    .map_or((allocated, at), |before| (before.allocated, before.at));
                files.insert(
                    key,
                    FileGrowth {
                        allocated,
                        at,
                        written,
                        position,
                        position_written,
                        write_only,
                        modified,
                        length,
                        append,
                        multiple: false,
                    },
                );
                let volume = volumes.entry(key.0).or_insert_with(|| VolumeWrites {
                    id: key.0,
                    path,
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
                if position_continuous && position.is_some() {
                    volume
                        .position_written_by_inode
                        .insert(key.1, position_written);
                }
                if !stream {
                    volume.receiving = true;
                    outputs.entry(key.0).or_insert_with(|| fd.path());
                }
                if rate > 0 {
                    volume.files.insert(key.1);
                }
                volume.written = volume.written.saturating_add(written);
                volume.rate = volume.rate.saturating_add(rate);
            }
        }
        self.files = files;
        self.remember(&mut volumes, &outputs);
        volumes.into_values().collect()
    }

    /// A many-file copy is between two files at most samples. While it
    /// holds no output open, the one volume it last sent output to still
    /// receives it.
    #[cfg(target_os = "linux")]
    fn remember(
        &mut self,
        volumes: &mut BTreeMap<u64, VolumeWrites>,
        outputs: &BTreeMap<u64, PathBuf>,
    ) {
        match outputs.iter().next() {
            None => {
                let Some((id, dir)) = self.last.as_ref() else {
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
            Some((id, output)) if outputs.len() == 1 => {
                // The descriptor closes soon; its directory stays measurable.
                let dir = std::fs::canonicalize(output)
                    .ok()
                    .and_then(|file| file.parent().map(PathBuf::from));
                if let Some(dir) = dir {
                    self.last = Some((*id, dir));
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
