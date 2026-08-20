use super::*;

pub(super) fn write_archive(
    destination: &Path,
    file: File,
    staging_root: &Path,
    manifest: &DevelopmentContextManifest,
) -> Result<(), DaemonError> {
    let mut manifest_writer = BoundedManifestWriter::new(MAX_MANIFEST_BYTES);
    serde_json::to_writer(&mut manifest_writer, manifest).map_err(|error| {
        context_error(format!("serialize development context manifest: {error}"))
    })?;
    let manifest_bytes = manifest_writer.into_inner();
    let mut archive_cleanup = FileCleanup::new(destination.to_path_buf());
    let encoder = GzBuilder::new().mtime(0).write(
        BoundedFileWriter::new(file, MAX_PACKAGE_BYTES),
        Compression::default(),
    );
    let mut archive = tar::Builder::new(encoder);
    append_archive_bytes(&mut archive, "manifest.json", &manifest_bytes, 0o600)?;
    append_staging_files(&mut archive, staging_root, staging_root)?;
    let encoder = archive
        .into_inner()
        .map_err(|error| context_io_error("finish development context tar", error))?;
    let mut file = encoder
        .finish()
        .map_err(|error| context_io_error("finish development context compression", error))?
        .into_inner();
    file.flush()
        .and_then(|_| file.sync_all())
        .map_err(|error| context_io_error("sync development context archive", error))?;
    archive_cleanup.keep();
    Ok(())
}

fn append_archive_bytes<W: Write>(
    archive: &mut tar::Builder<W>,
    path: &str,
    bytes: &[u8],
    mode: u32,
) -> Result<(), DaemonError> {
    append_archive_reader(archive, Path::new(path), bytes, bytes.len() as u64, mode)
}

fn append_archive_reader<W: Write, R: Read>(
    archive: &mut tar::Builder<W>,
    path: &Path,
    reader: R,
    size: u64,
    mode: u32,
) -> Result<(), DaemonError> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_size(size);
    header.set_mode(mode);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_cksum();
    archive
        .append_data(&mut header, path, reader)
        .map_err(|error| context_io_error("append development context artifact", error))
}

fn append_staging_files<W: Write>(
    archive: &mut tar::Builder<W>,
    root: &Path,
    current: &Path,
) -> Result<(), DaemonError> {
    let mut entries = fs::read_dir(current)
        .map_err(|error| context_io_error("enumerate development context staging", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| context_io_error("enumerate development context staging", error))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| context_io_error("inspect development context staging", error))?;
        if metadata.file_type().is_symlink() {
            return Err(context_error(format!(
                "development context staging contains symlink `{}`",
                path.display()
            )));
        }
        if metadata.is_dir() {
            append_staging_files(archive, root, &path)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| context_error("staging artifact escaped its root"))?;
            let file = File::open(&path)
                .map_err(|error| context_io_error("open development context artifact", error))?;
            append_archive_reader(archive, relative, file, metadata.len(), 0o600)?;
        } else {
            return Err(context_error(format!(
                "development context staging contains special file `{}`",
                path.display()
            )));
        }
    }
    Ok(())
}

struct BoundedManifestWriter {
    bytes: Vec<u8>,
    maximum_bytes: usize,
}

impl BoundedManifestWriter {
    fn new(maximum_bytes: usize) -> Self {
        Self {
            bytes: Vec::new(),
            maximum_bytes,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for BoundedManifestWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) > self.maximum_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "development context manifest exceeds {} bytes",
                    self.maximum_bytes
                ),
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct BoundedFileWriter {
    file: File,
    written_bytes: u64,
    maximum_bytes: u64,
}

impl BoundedFileWriter {
    fn new(file: File, maximum_bytes: u64) -> Self {
        Self {
            file,
            written_bytes: 0,
            maximum_bytes,
        }
    }

    fn into_inner(self) -> File {
        self.file
    }
}

impl Write for BoundedFileWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.written_bytes.saturating_add(bytes.len() as u64) > self.maximum_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "development context archive exceeds {} bytes",
                    self.maximum_bytes
                ),
            ));
        }
        let written = self.file.write(bytes)?;
        self.written_bytes = self.written_bytes.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

struct FileCleanup {
    path: PathBuf,
    remove: bool,
}

impl FileCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path, remove: true }
    }

    fn keep(&mut self) {
        self.remove = false;
    }
}

impl Drop for FileCleanup {
    fn drop(&mut self) {
        if self.remove {
            let _ = fs::remove_file(&self.path);
        }
    }
}
