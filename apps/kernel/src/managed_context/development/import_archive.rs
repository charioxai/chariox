use super::*;
use flate2::read::MultiGzDecoder;

struct ArtifactExpectation {
    sha256: String,
    size_bytes: u64,
}

pub(super) fn extract_and_verify_archive(
    archive_file: File,
    expected_project_id: &str,
    expected_source_repositories: Option<&[DevelopmentSourceRepositoryBinding]>,
    artifacts_root: &Path,
) -> Result<DevelopmentContextManifest, DaemonError> {
    let decoder = MultiGzDecoder::new(archive_file);
    let mut archive = StrictTarReader::new(decoder, MAX_DECOMPRESSED_ARCHIVE_BYTES);
    let mut manifest = None;
    let mut expected_artifacts = BTreeMap::new();
    let mut seen_artifacts = BTreeSet::new();
    let mut entry_count = 0_usize;
    while let Some(entry) = archive.next_entry()? {
        entry_count = entry_count.saturating_add(1);
        if entry_count > MAX_REPOSITORIES * (MAX_OVERLAY_FILES_PER_REPOSITORY * 2 + 2) + 1 {
            return Err(context_error(
                "development context archive contains too many entries",
            ));
        }
        let path = entry.path;
        validate_relative_path(&path)?;
        if entry_count == 1 {
            if path != "manifest.json" {
                return Err(context_error(
                    "development context archive must begin with manifest.json",
                ));
            }
            if entry.size > MAX_MANIFEST_BYTES as u64 {
                return Err(context_error(format!(
                    "development context manifest exceeds {MAX_MANIFEST_BYTES} bytes"
                )));
            }
            let bytes = archive.read_entry_bytes(entry.size, MAX_MANIFEST_BYTES as u64)?;
            let parsed: DevelopmentContextManifest =
                serde_json::from_slice(&bytes).map_err(|error| {
                    context_error(format!("parse development context manifest: {error}"))
                })?;
            expected_artifacts = validate_import_manifest(
                &parsed,
                expected_project_id,
                expected_source_repositories,
            )?;
            manifest = Some(parsed);
            continue;
        }
        let Some(expectation) = expected_artifacts.get(&path) else {
            return Err(context_error(format!(
                "development context archive contains unexpected artifact `{path}`"
            )));
        };
        if !seen_artifacts.insert(path.clone()) {
            return Err(context_error(format!(
                "development context archive repeats artifact `{path}`"
            )));
        }
        if entry.size != expectation.size_bytes {
            return Err(context_error(format!(
                "development context artifact `{path}` has an unexpected size"
            )));
        }
        let destination = artifacts_root.join(&path);
        if let Some(parent) = destination.parent() {
            create_private_directory(parent)?;
        }
        let mut file = private_create_new(&destination)?;
        let digest = archive.copy_entry(entry.size, &mut file)?;
        file.sync_all()
            .map_err(|error| context_io_error("sync development context artifact", error))?;
        if digest != expectation.sha256 {
            return Err(context_error(format!(
                "development context artifact `{path}` failed digest verification"
            )));
        }
    }
    archive.finish()?;
    let manifest =
        manifest.ok_or_else(|| context_error("development context manifest is missing"))?;
    if seen_artifacts.len() != expected_artifacts.len() {
        let missing = expected_artifacts
            .keys()
            .find(|path| !seen_artifacts.contains(*path))
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        return Err(context_error(format!(
            "development context archive is missing artifact `{missing}`"
        )));
    }
    Ok(manifest)
}

fn validate_import_manifest(
    manifest: &DevelopmentContextManifest,
    expected_project_id: &str,
    expected_source_repositories: Option<&[DevelopmentSourceRepositoryBinding]>,
) -> Result<BTreeMap<String, ArtifactExpectation>, DaemonError> {
    if manifest.schema_version != DEVELOPMENT_CONTEXT_SCHEMA_VERSION {
        return Err(context_error(format!(
            "unsupported development context schema version {}",
            manifest.schema_version
        )));
    }
    if manifest.project_id != expected_project_id {
        return Err(context_error(
            "development context project id does not match the expected project",
        ));
    }
    if let Some(expected) = expected_source_repositories {
        let matches = manifest.repositories.len() == expected.len()
            && manifest
                .repositories
                .iter()
                .zip(expected)
                .all(|(repository, binding)| {
                    repository.source_binding_sha256 == source_repository_binding_sha256(binding)
                });
        if !matches {
            return Err(context_error(
                "development context source repository selection does not match the launch plan",
            ));
        }
    }
    if manifest.repositories.is_empty() || manifest.repositories.len() > MAX_REPOSITORIES {
        return Err(context_error(
            "development context repository count is invalid",
        ));
    }
    if manifest
        .repositories
        .iter()
        .filter(|repository| repository.role == DevelopmentRepositoryRole::Primary)
        .count()
        != 1
    {
        return Err(context_error(
            "development context must contain exactly one primary repository",
        ));
    }
    let mut budget = ManifestMemoryBudget::new();
    budget.consume(manifest.project_id.len().saturating_add(256))?;
    let mut repository_ids = BTreeSet::new();
    let mut target_directories = BTreeSet::new();
    let mut artifacts = BTreeMap::new();
    let mut total_artifact_bytes = 0_u64;
    for repository in &manifest.repositories {
        if repository.source_binding_sha256.len() != 64
            || !repository
                .source_binding_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(context_error(
                "development context source repository binding is invalid",
            ));
        }
        if repository.repository_id.is_empty()
            || repository.repository_id.len() > 128
            || !repository
                .repository_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || !repository_ids.insert(repository.repository_id.clone())
        {
            return Err(context_error(
                "development context repository id is invalid",
            ));
        }
        validate_target_directory(&repository.target_directory)?;
        if !target_directories.insert(repository.target_directory.to_ascii_lowercase()) {
            return Err(context_error(
                "development context target directories must be unique",
            ));
        }
        validate_git_oid(&repository.head_sha)?;
        for value in [
            repository.logical_name.as_str(),
            repository.branch.as_deref().unwrap_or_default(),
            repository.upstream.as_deref().unwrap_or_default(),
            repository.origin_url.as_deref().unwrap_or_default(),
        ] {
            if value.len() > MAX_GIT_TEXT_BYTES
                || value
                    .chars()
                    .any(|character| character.is_control() || character == '\0')
            {
                return Err(context_error(
                    "development context repository metadata is invalid",
                ));
            }
        }
        if let Some(origin) = &repository.origin_url {
            if origin.chars().any(char::is_whitespace)
                || super::export::sanitize_origin_url(origin).as_deref() != Some(origin.as_str())
            {
                return Err(context_error(
                    "development context origin URL is not sanitized",
                ));
            }
        }
        if let Some(upstream) = &repository.upstream {
            if repository.branch.is_none()
                || repository.origin_url.is_none()
                || upstream
                    .strip_prefix("origin/")
                    .is_none_or(|branch| branch.is_empty())
            {
                return Err(context_error(
                    "development context upstream is not portable",
                ));
            }
        }
        budget.consume(
            repository
                .logical_name
                .len()
                .saturating_add(repository.target_directory.len())
                .saturating_add(repository.head_sha.len())
                .saturating_add(repository.branch.as_ref().map_or(0, String::len))
                .saturating_add(repository.upstream.as_ref().map_or(0, String::len))
                .saturating_add(repository.origin_url.as_ref().map_or(0, String::len))
                .saturating_add(2048),
        )?;
        let expected_bundle = format!(
            "repositories/{}/repository.bundle",
            repository.repository_id
        );
        if repository.bundle_path != expected_bundle
            || repository.bundle_size_bytes > MAX_BUNDLE_BYTES_PER_REPOSITORY
        {
            return Err(context_error(
                "development context Git bundle metadata is invalid",
            ));
        }
        validate_sha256(&repository.bundle_sha256)?;
        insert_artifact(
            &mut artifacts,
            &repository.bundle_path,
            &repository.bundle_sha256,
            repository.bundle_size_bytes,
        )?;
        total_artifact_bytes = total_artifact_bytes.saturating_add(repository.bundle_size_bytes);
        if repository.overlay.len() > MAX_OVERLAY_FILES_PER_REPOSITORY {
            return Err(context_error(
                "development context overlay has too many paths",
            ));
        }
        let mut overlay_paths = BTreeSet::new();
        let mut overlay_objects = BTreeMap::new();
        for entry in &repository.overlay {
            validate_relative_path(&entry.path)?;
            if super::overlay::context_force_excluded_path(&entry.path) {
                return Err(context_error(format!(
                    "development context overlay path `{}` is reserved or excluded",
                    entry.path
                )));
            }
            if !overlay_paths.insert(entry.path.clone()) {
                return Err(context_error("development context overlay repeats a path"));
            }
            budget.consume(entry.path.len().saturating_add(256))?;
            for state in [&entry.index, &entry.worktree] {
                if let DevelopmentFileState::File {
                    object_path,
                    sha256,
                    size_bytes,
                    ..
                } = state
                {
                    validate_sha256(sha256)?;
                    if *size_bytes > MAX_OVERLAY_FILE_BYTES
                        || object_path
                            != &format!(
                                "repositories/{}/objects/{sha256}",
                                repository.repository_id
                            )
                    {
                        return Err(context_error(
                            "development context overlay object metadata is invalid",
                        ));
                    }
                    budget.consume(file_state_manifest_bytes_for_import(state))?;
                    match overlay_objects.get(object_path) {
                        Some((known_sha, known_size))
                            if known_sha != sha256 || known_size != size_bytes =>
                        {
                            return Err(context_error(
                                "development context overlay object metadata conflicts",
                            ));
                        }
                        Some(_) => {}
                        None => {
                            overlay_objects
                                .insert(object_path.clone(), (sha256.clone(), *size_bytes));
                        }
                    }
                }
            }
        }
        let overlay_bytes = overlay_objects
            .values()
            .fold(0_u64, |total, (_, size)| total.saturating_add(*size));
        if overlay_bytes != repository.overlay_size_bytes
            || overlay_bytes > MAX_OVERLAY_BYTES_PER_REPOSITORY
        {
            return Err(context_error(
                "development context overlay size metadata is invalid",
            ));
        }
        for (path, (sha256, size_bytes)) in overlay_objects {
            insert_artifact(&mut artifacts, &path, &sha256, size_bytes)?;
        }
        total_artifact_bytes = total_artifact_bytes.saturating_add(overlay_bytes);
        if total_artifact_bytes > MAX_PACKAGE_BYTES {
            return Err(context_error(
                "development context artifact total exceeds the package limit",
            ));
        }
    }
    Ok(artifacts)
}

struct StrictTarReader<R> {
    reader: HardLimitReader<R>,
    pending_long_name: Option<String>,
    finished: bool,
}

struct StrictTarEntry {
    path: String,
    size: u64,
}

impl<R: Read> StrictTarReader<R> {
    fn new(reader: R, maximum_bytes: u64) -> Self {
        Self {
            reader: HardLimitReader::new(reader, maximum_bytes),
            pending_long_name: None,
            finished: false,
        }
    }

    fn next_entry(&mut self) -> Result<Option<StrictTarEntry>, DaemonError> {
        if self.finished {
            return Ok(None);
        }
        loop {
            let mut header = [0_u8; 512];
            self.reader
                .read_exact(&mut header)
                .map_err(|error| context_io_error("read development context tar header", error))?;
            if header.iter().all(|byte| *byte == 0) {
                let mut second = [0_u8; 512];
                self.reader.read_exact(&mut second).map_err(|error| {
                    context_io_error("read development context tar terminator", error)
                })?;
                if !second.iter().all(|byte| *byte == 0) {
                    return Err(context_error(
                        "development context archive has an invalid tar terminator",
                    ));
                }
                self.finished = true;
                return Ok(None);
            }
            validate_tar_checksum(&header)?;
            let size = parse_tar_octal(&header[124..136], "entry size")?;
            match header[156] {
                0 | b'0' => {
                    let path = match self.pending_long_name.take() {
                        Some(path) => path,
                        None => tar_header_path(&header)?,
                    };
                    return Ok(Some(StrictTarEntry { path, size }));
                }
                b'L' => {
                    if self.pending_long_name.is_some() || size == 0 {
                        return Err(context_error(
                            "development context archive has invalid GNU path metadata",
                        ));
                    }
                    let mut bytes = self.read_entry_bytes(size, MAX_ARCHIVE_PATH_BYTES as u64)?;
                    while bytes.last() == Some(&0) {
                        bytes.pop();
                    }
                    let path = String::from_utf8(bytes).map_err(|_| {
                        context_error("development context archive path is not valid UTF-8")
                    })?;
                    if path.is_empty() {
                        return Err(context_error(
                            "development context archive has empty GNU path metadata",
                        ));
                    }
                    self.pending_long_name = Some(path);
                }
                b'x' | b'g' | b'K' => {
                    return Err(context_error(
                        "development context archive contains unsupported tar extension metadata",
                    ));
                }
                _ => {
                    return Err(context_error(
                        "development context archive may contain only regular files",
                    ));
                }
            }
        }
    }

    fn read_entry_bytes(&mut self, size: u64, maximum: u64) -> Result<Vec<u8>, DaemonError> {
        if size > maximum || size > usize::MAX as u64 {
            return Err(context_error(format!(
                "development context tar entry exceeds {maximum} bytes"
            )));
        }
        let mut bytes = vec![0_u8; size as usize];
        self.reader
            .read_exact(&mut bytes)
            .map_err(|error| context_io_error("read development context tar entry", error))?;
        self.read_padding(size)?;
        Ok(bytes)
    }

    fn copy_entry(&mut self, size: u64, destination: &mut File) -> Result<String, DaemonError> {
        let mut remaining = size;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        while remaining > 0 {
            let wanted = remaining.min(buffer.len() as u64) as usize;
            self.reader
                .read_exact(&mut buffer[..wanted])
                .map_err(|error| context_io_error("read development context artifact", error))?;
            destination
                .write_all(&buffer[..wanted])
                .map_err(|error| context_io_error("extract development context artifact", error))?;
            hasher.update(&buffer[..wanted]);
            remaining -= wanted as u64;
        }
        self.read_padding(size)?;
        Ok(format!("{:x}", hasher.finalize()))
    }

    fn read_padding(&mut self, size: u64) -> Result<(), DaemonError> {
        let padding = (512 - size % 512) % 512;
        let mut bytes = [0_u8; 511];
        self.reader
            .read_exact(&mut bytes[..padding as usize])
            .map_err(|error| context_io_error("read development context tar padding", error))?;
        if bytes[..padding as usize].iter().any(|byte| *byte != 0) {
            return Err(context_error(
                "development context archive has nonzero tar padding",
            ));
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), DaemonError> {
        if !self.finished || self.pending_long_name.is_some() {
            return Err(context_error(
                "development context archive ended before its tar terminator",
            ));
        }
        let mut buffer = [0_u8; 4096];
        loop {
            let read = self
                .reader
                .read(&mut buffer)
                .map_err(|error| context_io_error("finish development context archive", error))?;
            if read == 0 {
                return Ok(());
            }
            if buffer[..read].iter().any(|byte| *byte != 0) {
                return Err(context_error(
                    "development context archive has trailing tar data",
                ));
            }
        }
    }
}

struct HardLimitReader<R> {
    inner: R,
    read_bytes: u64,
    maximum_bytes: u64,
}

impl<R> HardLimitReader<R> {
    fn new(inner: R, maximum_bytes: u64) -> Self {
        Self {
            inner,
            read_bytes: 0,
            maximum_bytes,
        }
    }
}

impl<R: Read> Read for HardLimitReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.read_bytes >= self.maximum_bytes {
            let mut byte = [0_u8; 1];
            return match self.inner.read(&mut byte)? {
                0 => Ok(0),
                _ => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "development context archive exceeds {} decompressed bytes",
                        self.maximum_bytes
                    ),
                )),
            };
        }
        let allowed = (self.maximum_bytes - self.read_bytes).min(buffer.len() as u64) as usize;
        let read = self.inner.read(&mut buffer[..allowed])?;
        self.read_bytes = self.read_bytes.saturating_add(read as u64);
        Ok(read)
    }
}

fn validate_tar_checksum(header: &[u8; 512]) -> Result<(), DaemonError> {
    let expected = parse_tar_octal(&header[148..156], "header checksum")?;
    let actual = header
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            if (148..156).contains(&index) {
                b' ' as u64
            } else {
                *byte as u64
            }
        })
        .sum::<u64>();
    if actual != expected {
        return Err(context_error(
            "development context archive has an invalid tar checksum",
        ));
    }
    Ok(())
}

fn parse_tar_octal(bytes: &[u8], label: &str) -> Result<u64, DaemonError> {
    if bytes.first().is_some_and(|byte| byte & 0x80 != 0) {
        return Err(context_error(format!(
            "development context tar {label} uses an unsupported numeric encoding"
        )));
    }
    let text = bytes
        .iter()
        .copied()
        .skip_while(|byte| *byte == 0 || *byte == b' ')
        .take_while(|byte| *byte != 0 && *byte != b' ')
        .collect::<Vec<_>>();
    if text.is_empty() {
        return Ok(0);
    }
    let text = std::str::from_utf8(&text)
        .map_err(|_| context_error(format!("development context tar {label} is invalid")))?;
    u64::from_str_radix(text, 8)
        .map_err(|_| context_error(format!("development context tar {label} is invalid")))
}

fn tar_header_path(header: &[u8; 512]) -> Result<String, DaemonError> {
    let name = tar_text_field(&header[0..100])?;
    let prefix = tar_text_field(&header[345..500])?;
    let path = if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    };
    if path.len() > MAX_ARCHIVE_PATH_BYTES {
        return Err(context_error(format!(
            "development context archive path exceeds {MAX_ARCHIVE_PATH_BYTES} bytes"
        )));
    }
    Ok(path)
}

fn tar_text_field(bytes: &[u8]) -> Result<String, DaemonError> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8(bytes[..end].to_vec())
        .map_err(|_| context_error("development context archive path is not valid UTF-8"))
}

fn insert_artifact(
    artifacts: &mut BTreeMap<String, ArtifactExpectation>,
    path: &str,
    sha256: &str,
    size_bytes: u64,
) -> Result<(), DaemonError> {
    validate_relative_path(path)?;
    match artifacts.get(path) {
        Some(existing) if existing.sha256 != sha256 || existing.size_bytes != size_bytes => Err(
            context_error("development context artifact metadata conflicts"),
        ),
        Some(_) => Ok(()),
        None => {
            artifacts.insert(
                path.to_string(),
                ArtifactExpectation {
                    sha256: sha256.to_string(),
                    size_bytes,
                },
            );
            Ok(())
        }
    }
}

fn validate_target_directory(value: &str) -> Result<(), DaemonError> {
    if value.is_empty()
        || value.len() > 255
        || Path::new(value).components().count() != 1
        || !Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(context_error(
            "development context target directory is invalid",
        ));
    }
    Ok(())
}

pub(super) fn validate_git_oid(value: &str) -> Result<(), DaemonError> {
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(context_error(
            "development context Git object id is invalid",
        ));
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<(), DaemonError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(context_error(
            "development context SHA-256 digest is invalid",
        ));
    }
    Ok(())
}

fn file_state_manifest_bytes_for_import(state: &DevelopmentFileState) -> usize {
    match state {
        DevelopmentFileState::Absent => 32,
        DevelopmentFileState::File {
            object_path,
            sha256,
            ..
        } => object_path
            .len()
            .saturating_add(sha256.len())
            .saturating_add(192),
    }
}
