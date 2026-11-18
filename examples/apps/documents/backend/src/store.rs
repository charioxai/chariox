//! The document files: private files in the App's own data directory (`data/documents/<id>/`),
//! plus the staged copies the file broker imports from and offers.

use crate::model::{self, Doc, FileWrite, Prune};
use chariox_app_sdk::{app_types::AppError, error, files};
use std::{
    collections::hash_map::RandomState,
    fs,
    hash::{BuildHasher, Hasher},
    io::ErrorKind,
    path::{Path, PathBuf},
};

const DATA: &str = "data";

pub fn on_disk(path: &str) -> PathBuf {
    Path::new(DATA).join(path)
}

/// Eight random hex digits; the WASI random source, so no two writes share a name.
pub fn nonce() -> String {
    format!("{:08x}", RandomState::new().build_hasher().finish() as u32)
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

pub fn stamp() -> model::Stamp {
    model::Stamp {
        now_ms: now_ms(),
        nonce: nonce(),
    }
}

/// The broker's atomic replace does not create parents.
pub async fn replace(path: &str, content: Vec<u8>) -> Result<(), AppError> {
    if let Some(parent) = on_disk(path).parent() {
        fs::create_dir_all(parent).map_err(|e| io("create the folder for", path, &e))?;
    }
    let (mut writer, reader) = chariox_app_sdk::bindings::wit_stream::new();
    let (result, remaining) =
        futures::join!(files::atomic_replace(path.to_owned(), reader), async move {
            writer.write_all(content).await
        });
    result?;
    if !remaining.is_empty() {
        return Err(error("FILE_WRITE_FAILED", "file broker stopped reading"));
    }
    Ok(())
}

pub async fn write(file: &FileWrite) -> Result<(), AppError> {
    replace(&file.path, file.content.clone().into_bytes()).await
}

/// Removes a file no index names, and its folder when that leaves it empty.
pub fn discard(file: &FileWrite) {
    let path = on_disk(&file.path);
    let _ = fs::remove_file(&path);
    if let Some(folder) = path.parent() {
        let _ = fs::remove_dir(folder);
    }
}

pub fn read(doc: &Doc, revision: u64) -> Result<String, AppError> {
    let file = doc
        .file_of(revision)
        .ok_or_else(|| error("NOT_FOUND", format!("No revision {revision}")))?;
    fs::read_to_string(on_disk(&doc.content_path(file))).map_err(|e| match e.kind() {
        // A concurrent save or delete pruned it after this index was read.
        ErrorKind::NotFound => error("CONFLICT", "Document changed; reload and try again"),
        _ => io("read", file, &e),
    })
}

/// A file that is not a document revision: `None` while nothing was written.
pub fn read_private(path: &str) -> Result<Option<String>, AppError> {
    match fs::read_to_string(on_disk(path)) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io("read", path, &e)),
    }
}

/// Every file of a private folder, as text; empty while nothing was written there.
pub fn read_private_folder(folder: &str) -> Result<Vec<String>, AppError> {
    let entries = match fs::read_dir(on_disk(folder)) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io("list", folder, &e)),
    };
    let mut texts = Vec::new();
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        // A file removed after the listing was discarded by its window: nothing to hand over.
        if let Some(text) = read_private(&format!("{folder}/{name}"))? {
            texts.push(text);
        }
    }
    Ok(texts)
}

/// The paths of the files in a private folder; empty while nothing was written there.
pub fn private_files(folder: &str) -> Result<Vec<String>, AppError> {
    let entries = match fs::read_dir(on_disk(folder)) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io("list", folder, &e)),
    };
    Ok(entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .map(|name| format!("{folder}/{name}"))
        .collect())
}

pub fn remove_private(path: &str) -> Result<(), AppError> {
    match fs::remove_file(on_disk(path)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io("remove", path, &e)),
    }
}

/// Best effort: a file the index no longer names is never read, so a leftover is harmless.
pub fn prune(prune: &Prune) {
    match prune {
        Prune::Folder(id) => {
            let _ = fs::remove_dir_all(on_disk(&format!("documents/{id}")));
        }
        Prune::Older { id, keep } => {
            let Ok(entries) = fs::read_dir(on_disk(&format!("documents/{id}"))) else {
                return;
            };
            let names: Vec<String> = entries
                .flatten()
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect();
            for name in model::stale(&names, keep) {
                let _ = fs::remove_file(on_disk(&format!("documents/{id}/{name}")));
            }
        }
    }
}

fn io(action: &str, what: &str, e: &std::io::Error) -> AppError {
    error("FILE_FAILED", format!("cannot {action} {what}: {e}"))
}
