//! MP-08 / MP-11: exact read-only Node runtime assets, including distro builtins.
use super::*;
use std::io::{Read, Seek, SeekFrom};

/// The operator-fixed runtime is discovered once per canonical path, size and
/// modification time; a changed runtime or a vanished asset is rediscovered.
pub(super) fn runtime_files(node: &Path) -> Result<BTreeSet<PathBuf>, crate::DaemonError> {
    type Discovered = BTreeMap<PathBuf, (u64, std::time::SystemTime, BTreeSet<PathBuf>)>;
    static DISCOVERED: std::sync::Mutex<Discovered> = std::sync::Mutex::new(BTreeMap::new());
    let canonical = fs::canonicalize(node).map_err(io_error("workflow_code.compile"))?;
    let metadata = fs::metadata(&canonical).map_err(io_error("workflow_code.compile"))?;
    let identity = (
        metadata.len(),
        metadata
            .modified()
            .map_err(io_error("workflow_code.compile"))?,
    );
    if let Some((len, modified, files)) = DISCOVERED.lock().unwrap().get(&canonical) {
        if (*len, *modified) == identity
            && files
                .iter()
                .all(|path| fs::metadata(path).is_ok_and(|metadata| metadata.is_file()))
        {
            return Ok(files.clone());
        }
    }
    let files = discover_runtime_files(node)?;
    DISCOVERED
        .lock()
        .unwrap()
        .insert(canonical, (identity.0, identity.1, files.clone()));
    Ok(files)
}

fn discover_runtime_files(node: &Path) -> Result<BTreeSet<PathBuf>, crate::DaemonError> {
    let dependencies = Command::new("/usr/bin/ldd")
        .env_clear()
        .arg(node)
        .output()
        .map_err(io_error("workflow_code.compile"))?;
    if !dependencies.status.success() {
        return Err(isolation_error(
            "cannot identify compiler runtime libraries",
        ));
    }
    let mut files = BTreeSet::new();
    for word in String::from_utf8_lossy(&dependencies.stdout).split_whitespace() {
        if word.starts_with('/') {
            let path = PathBuf::from(word);
            if !fs::metadata(&path).is_ok_and(|metadata| metadata.is_file()) {
                return Err(isolation_error("compiler runtime library is unavailable"));
            }
            files.insert(path);
        }
    }
    // Distro builds put NODE_SHARED_BUILTIN_* path literals in libnode rather
    // than embedding those modules. Read the selected runtime's ELF data; never
    // execute workflow code or expose /usr/share/nodejs (or any other directory).
    let mut binaries = vec![node.to_path_buf()];
    binaries.extend(
        files
            .iter()
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("libnode.so"))
            })
            .cloned(),
    );
    for binary in binaries {
        files.extend(external_data_files(&binary)?);
        if files.len() > 256 {
            return Err(isolation_error(
                "compiler runtime dependency limit exceeded",
            ));
        }
    }
    Ok(files)
}

fn runtime_data_path(bytes: &[u8]) -> Option<PathBuf> {
    let name = std::str::from_utf8(bytes).ok()?;
    let path = Path::new(name);
    if !path.is_absolute()
        || path
            .components()
            .skip(1)
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return None;
    }
    // These are Node's shareable builtins, including Debian's additional ones.
    // Match complete package-relative names, never arbitrary JS/data paths.
    let builtin = [
        "cjs-module-lexer/lexer.js",
        "cjs-module-lexer/dist/lexer.js",
        "undici/undici.js",
        "undici/undici-fetch.js",
        "acorn/dist/acorn.js",
        "acorn-walk/dist/walk.js",
        "minimatch/dist/cjs/index.bundle.js",
        "amaro/dist/index.js",
    ]
    .iter()
    .any(|suffix| path.ends_with(suffix));
    let icu = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.strip_prefix("icudt")
                .and_then(|name| name.strip_suffix(".dat"))
                .and_then(|name| name.strip_suffix(['l', 'b']))
                .is_some_and(|version| {
                    !version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit())
                })
        });
    (builtin || icu).then(|| path.to_path_buf())
}

/// Scan only file-backed, non-writable ELF load segments (read-only and, where
/// .rodata shares the text segment as on arm64, read+execute) for bounded NUL path
/// literals. Stripped distro binaries retain these runtime names. No binutils,
/// Node execution, directory traversal or package-manager database is needed.
fn external_data_files(binary: &Path) -> Result<BTreeSet<PathBuf>, crate::DaemonError> {
    let invalid = || isolation_error("compiler runtime is not a supported ELF binary");
    let mut file = fs::File::open(binary).map_err(io_error("workflow_code.compile"))?;
    let size = file
        .metadata()
        .map_err(io_error("workflow_code.compile"))?
        .len();
    let mut header = [0; 64];
    file.read_exact(&mut header).map_err(|_| invalid())?;
    if &header[..4] != b"\x7fELF" || ![1, 2].contains(&header[4]) || ![1, 2].contains(&header[5]) {
        return Err(invalid());
    }
    let wide = header[4] == 2;
    let little = header[5] == 1;
    let short = |bytes: &[u8], at: usize| {
        let value = bytes[at..at + 2].try_into().unwrap();
        if little {
            u16::from_le_bytes(value)
        } else {
            u16::from_be_bytes(value)
        }
    };
    let word = |bytes: &[u8], at: usize| {
        let value = bytes[at..at + 4].try_into().unwrap();
        if little {
            u32::from_le_bytes(value)
        } else {
            u32::from_be_bytes(value)
        }
    };
    let long = |bytes: &[u8], at: usize| {
        let value = bytes[at..at + 8].try_into().unwrap();
        if little {
            u64::from_le_bytes(value)
        } else {
            u64::from_be_bytes(value)
        }
    };
    let offset = if wide {
        long(&header, 32)
    } else {
        u64::from(word(&header, 28))
    };
    let entry_size = short(&header, if wide { 54 } else { 42 }) as usize;
    let count = short(&header, if wide { 56 } else { 44 }) as usize;
    if entry_size != if wide { 56 } else { 32 }
        || count == 0
        || count > 128
        || offset
            .checked_add((entry_size * count) as u64)
            .is_none_or(|end| end > size)
    {
        return Err(invalid());
    }
    let mut segments = vec![0; entry_size * count];
    file.seek(SeekFrom::Start(offset))
        .and_then(|_| file.read_exact(&mut segments))
        .map_err(|_| invalid())?;
    let mut files = BTreeSet::new();
    let mut total = 0u64;
    for segment in segments.chunks_exact(entry_size) {
        let flags = word(segment, if wide { 4 } else { 24 });
        if word(segment, 0) != 1 || flags & 2 != 0 {
            continue;
        }
        let offset = if wide {
            long(segment, 8)
        } else {
            u64::from(word(segment, 4))
        };
        let length = if wide {
            long(segment, 32)
        } else {
            u64::from(word(segment, 16))
        };
        total = total.saturating_add(length);
        if total > 256 * 1024 * 1024 || offset.checked_add(length).is_none_or(|end| end > size) {
            return Err(invalid());
        }
        file.seek(SeekFrom::Start(offset))
            .map_err(io_error("workflow_code.compile"))?;
        let mut remaining = length;
        let mut buffer = [0; 64 * 1024];
        let mut literal = Vec::new();
        let mut oversized = false;
        while remaining > 0 {
            let read = remaining.min(buffer.len() as u64) as usize;
            file.read_exact(&mut buffer[..read])
                .map_err(|_| invalid())?;
            remaining -= read as u64;
            for byte in &buffer[..read] {
                if *byte == 0 {
                    if !oversized {
                        if let Some(path) = runtime_data_path(&literal) {
                            if !fs::metadata(&path).is_ok_and(|metadata| metadata.is_file()) {
                                return Err(isolation_error(
                                    "compiler runtime builtin or ICU data is unavailable",
                                ));
                            }
                            files.insert(path);
                            if files.len() > 256 {
                                return Err(isolation_error(
                                    "compiler runtime dependency limit exceeded",
                                ));
                            }
                        }
                    }
                    literal.clear();
                    oversized = false;
                } else if !oversized {
                    // Ignore non-path strings without buffering embedded source.
                    if (literal.is_empty() && *byte != b'/') || literal.len() == 4096 {
                        literal.clear();
                        oversized = true;
                    } else {
                        literal.push(*byte);
                    }
                }
            }
        }
    }
    Ok(files)
}

#[cfg(test)]
#[path = "compiler_linux_tests.rs"]
mod tests;
