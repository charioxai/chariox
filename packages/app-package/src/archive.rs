use std::collections::BTreeMap;

use tar::{EntryType, Header};

use crate::{ErrorCode, Limits, PackageError, Result};

const BLOCK: usize = 512;
pub(crate) const CONTROL_PATHS: [&str; 3] = [
    "manifest.json",
    "integrity.json",
    "signatures/publisher.sig",
];

pub(crate) struct Entry<'a> {
    pub path: String,
    pub data: &'a [u8],
}

/// Only a canonical, uncompressed USTAR regular-file subset is accepted. This
/// parser never calls unpack(), follows links, creates files, or decompresses.
pub(crate) fn parse<'a>(bytes: &'a [u8], limits: &Limits) -> Result<Vec<Entry<'a>>> {
    if bytes.len() > limits.max_archive_bytes {
        return Err(PackageError::new(
            ErrorCode::ArchiveLimit,
            "archive exceeds byte limit",
        ));
    }
    if bytes.len() < BLOCK * 2 || !bytes.len().is_multiple_of(BLOCK) {
        return Err(PackageError::new(
            ErrorCode::InvalidArchive,
            "expected uncompressed USTAR blocks",
        ));
    }
    let mut offset = 0usize;
    let mut entries = Vec::new();
    let mut paths = PathSet::default();
    loop {
        let raw = bytes.get(offset..offset + BLOCK).ok_or_else(|| {
            PackageError::new(ErrorCode::InvalidArchive, "missing end-of-archive blocks")
        })?;
        if raw.iter().all(|byte| *byte == 0) {
            if bytes.len() - offset != BLOCK * 2 || bytes[offset..].iter().any(|byte| *byte != 0) {
                return Err(PackageError::new(
                    ErrorCode::InvalidArchive,
                    "archive must end with exactly two zero blocks",
                ));
            }
            break;
        }
        if entries.len() >= limits.max_entries {
            return Err(PackageError::new(
                ErrorCode::ArchiveLimit,
                "archive exceeds entry limit",
            ));
        }
        let header = Header::from_byte_slice(raw);
        if !header.entry_type().is_file() || header.as_ustar().is_none() {
            return Err(PackageError::new(
                ErrorCode::InvalidArchive,
                "only USTAR regular files are supported",
            ));
        }
        let path_bytes = header.path_bytes();
        let path = std::str::from_utf8(&path_bytes)
            .map_err(|_| PackageError::new(ErrorCode::InvalidPath, "path must be UTF-8"))?;
        paths.insert(path, limits)?;
        let size = header
            .size()
            .map_err(|_| PackageError::new(ErrorCode::InvalidArchive, "invalid USTAR file size"))?;
        if size > limits.max_file_bytes as u64 {
            return Err(PackageError::new(
                ErrorCode::ArchiveLimit,
                "file exceeds byte limit",
            ));
        }
        if canonical_header(path, size)?.as_bytes() != raw {
            return Err(PackageError::new(
                ErrorCode::InvalidArchive,
                "noncanonical USTAR header or checksum",
            ));
        }
        let size = usize::try_from(size).map_err(|_| {
            PackageError::new(
                ErrorCode::ArchiveLimit,
                "file size does not fit this platform",
            )
        })?;
        let data_start = offset + BLOCK;
        let data_end = data_start
            .checked_add(size)
            .ok_or_else(|| PackageError::new(ErrorCode::ArchiveLimit, "file size overflow"))?;
        let padding = (BLOCK - size % BLOCK) % BLOCK;
        let next = data_end.checked_add(padding).ok_or_else(|| {
            PackageError::new(ErrorCode::ArchiveLimit, "padded file size overflow")
        })?;
        let data = bytes
            .get(data_start..data_end)
            .ok_or_else(|| PackageError::new(ErrorCode::InvalidArchive, "truncated file"))?;
        if bytes
            .get(data_end..next)
            .is_none_or(|pad| pad.iter().any(|b| *b != 0))
        {
            return Err(PackageError::new(
                ErrorCode::InvalidArchive,
                "missing or nonzero file padding",
            ));
        }
        entries.push(Entry {
            path: path.to_owned(),
            data,
        });
        offset = next;
    }
    if entries.len() < CONTROL_PATHS.len()
        || entries
            .iter()
            .take(3)
            .map(|entry| entry.path.as_str())
            .ne(CONTROL_PATHS)
    {
        return Err(PackageError::new(
            ErrorCode::InvalidArchive,
            "missing or reordered package control files",
        ));
    }
    if entries[3..]
        .windows(2)
        .any(|pair| pair[0].path >= pair[1].path)
    {
        return Err(PackageError::new(
            ErrorCode::InvalidArchive,
            "payload files must be sorted by path",
        ));
    }
    Ok(entries)
}

pub(crate) fn encode(entries: &[(&str, &[u8])], limits: &Limits) -> Result<Vec<u8>> {
    if entries.len() > limits.max_entries {
        return Err(PackageError::new(
            ErrorCode::ArchiveLimit,
            "archive exceeds entry limit",
        ));
    }
    let mut length = BLOCK * 2;
    let mut paths = PathSet::default();
    for (path, data) in entries {
        paths.insert(path, limits)?;
        if data.len() > limits.max_file_bytes {
            return Err(PackageError::new(
                ErrorCode::ArchiveLimit,
                "file exceeds byte limit",
            ));
        }
        length = length
            .checked_add(BLOCK)
            .and_then(|n| n.checked_add(data.len()))
            .and_then(|n| n.checked_add((BLOCK - data.len() % BLOCK) % BLOCK))
            .filter(|n| *n <= limits.max_archive_bytes)
            .ok_or_else(|| {
                PackageError::new(ErrorCode::ArchiveLimit, "archive exceeds byte limit")
            })?;
    }
    let mut bytes = Vec::with_capacity(length);
    for (path, data) in entries {
        bytes.extend_from_slice(canonical_header(path, data.len() as u64)?.as_bytes());
        bytes.extend_from_slice(data);
        bytes.resize(bytes.len() + (BLOCK - data.len() % BLOCK) % BLOCK, 0);
    }
    bytes.resize(length, 0);
    Ok(bytes)
}

fn canonical_header(path: &str, size: u64) -> Result<Header> {
    let mut header = Header::new_ustar();
    header.set_path(path).map_err(|_| {
        PackageError::new(
            ErrorCode::InvalidPath,
            "path cannot be represented in USTAR",
        )
    })?;
    header.set_entry_type(EntryType::Regular);
    header.set_mode(0o644);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_size(size);
    header.set_cksum();
    Ok(header)
}

pub(crate) fn validate_path(path: &str, limits: &Limits) -> Result<()> {
    let invalid = || {
        PackageError::new(
            ErrorCode::InvalidPath,
            "path must be a portable relative package path",
        )
    };
    if path.is_empty()
        || path.len() > limits.max_path_bytes
        || !path.is_ascii()
        || path
            .bytes()
            .any(|b| b < 32 || b == 127 || b"<>:\\\"|?*".contains(&b))
    {
        return Err(invalid());
    }
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() > limits.max_path_depth {
        return Err(PackageError::new(
            ErrorCode::ArchiveLimit,
            "path exceeds depth limit",
        ));
    }
    for part in parts {
        if part.is_empty() || part == "." || part == ".." || part.ends_with(['.', ' ']) {
            return Err(invalid());
        }
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if ["con", "prn", "aux", "nul"].contains(&stem.as_str())
            || (stem.len() == 4
                && (stem.starts_with("com") || stem.starts_with("lpt"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err(invalid());
        }
    }
    Ok(())
}

#[derive(Default)]
struct PathSet {
    // Directory names are tracked too: Ui/a + ui/b collide on macOS.
    entries: BTreeMap<String, (String, bool)>,
}

impl PathSet {
    fn insert(&mut self, path: &str, limits: &Limits) -> Result<()> {
        validate_path(path, limits)?;
        for end in path
            .match_indices('/')
            .map(|(i, _)| i)
            .chain(std::iter::once(path.len()))
        {
            let prefix = &path[..end];
            let is_file = end == path.len();
            let key = prefix.to_ascii_lowercase();
            if let Some((old, old_file)) = self.entries.get(&key) {
                if old != prefix || *old_file || is_file {
                    return Err(PackageError::new(
                        ErrorCode::DuplicatePath,
                        "duplicate, case-colliding, or file/directory-conflicting path",
                    ));
                }
            } else {
                self.entries.insert(key, (prefix.to_owned(), is_file));
            }
        }
        Ok(())
    }
}
