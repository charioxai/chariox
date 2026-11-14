//! The diagram files the App reads and writes through the kernel's file broker: private files in
//! its own data directory, user-picked files imported by grant and exported copies offered by grant.

use crate::{export_cleanup, scene};
use chariox_app_sdk::{app_types::AppError, error, files, host};
use serde_json::{json, Value};
use std::{fs::File, io::Read};

struct ImportFile(String);
impl ImportFile {
    fn reserve() -> Result<Self, AppError> {
        for index in 0..1024 {
            let name = format!("import-{index}.excalidraw");
            match File::create_new(format!("data/{name}")) {
                Ok(_) => return Ok(Self(name)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => {
                    return Err(error(
                        "IMPORT_FAILED",
                        "cannot reserve a private import file",
                    ))
                }
            }
        }
        Err(error(
            "IMPORT_FAILED",
            "too many concurrent imports; retry after they finish",
        ))
    }
}
impl Drop for ImportFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(format!("data/{}", self.0));
    }
}

fn reserve_export(name: &str) -> Result<String, AppError> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| error("EXPORT_FAILED", "cannot read the export staging clock"))?
        .as_nanos();
    for index in 0..1024 {
        // Named saves cannot contain paths or start with a dot. Reserve a
        // directory so the broker still offers the person's chosen filename.
        let directory = format!(".export-{stamp:x}-{index}");
        match std::fs::create_dir(format!("data/{directory}")) {
            Ok(_) => return Ok(format!("{directory}/{name}")),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => {
                return Err(error(
                    "EXPORT_FAILED",
                    "cannot reserve a private export file",
                ))
            }
        }
    }
    Err(error(
        "EXPORT_FAILED",
        "too many concurrent exports; retry after they finish",
    ))
}

struct ExportFile(String);
impl ExportFile {
    fn reserve(name: &str) -> Result<Self, AppError> {
        reserve_export(name).map(Self)
    }

    fn remove(&self) -> Result<(), AppError> {
        std::fs::remove_file(format!("data/{}", self.0)).map_err(|_| {
            error(
                "FILE_CLEANUP_FAILED",
                "copy was offered but its private staging file could not be removed",
            )
        })?;
        std::fs::remove_dir(format!("data/{}", self.0.rsplit_once('/').unwrap().0)).map_err(|_| {
            error(
                "FILE_CLEANUP_FAILED",
                "copy was offered but its private staging directory could not be removed",
            )
        })
    }
}
impl Drop for ExportFile {
    fn drop(&mut self) {
        // Failed writes, failed offers and cancellation must release their reservation too.
        let _ = export_cleanup::remove(std::path::Path::new(&format!("data/{}", self.0)));
    }
}

async fn replace(name: String, bytes: Vec<u8>) -> Result<u64, AppError> {
    let (mut writer, reader) = chariox_app_sdk::bindings::wit_stream::new();
    let (result, remaining) = futures::join!(files::atomic_replace(name, reader), async move {
        writer.write_all(bytes).await
    });
    let written = result?;
    if !remaining.is_empty() {
        return Err(error("FILE_WRITE_FAILED", "file broker stopped reading"));
    }
    Ok(written)
}

/// `write_file`: a private diagram file, or a copy offered to the person by grant.
pub async fn write(input: &Value) -> Result<Value, AppError> {
    let name = scene::filename(&input["name"])?;
    let bytes = scene::file_bytes(input)?;
    if name.ends_with(".excalidraw") || name.ends_with(".json") {
        scene::parse(
            std::str::from_utf8(&bytes)
                .map_err(|_| error("INVALID_SCENE", "diagram must be UTF-8"))?,
        )?;
    }
    let offer = input["offer"].as_bool().unwrap_or(false);
    let staging = offer.then(|| ExportFile::reserve(&name)).transpose()?;
    let destination = staging
        .as_ref()
        .map_or_else(|| name.clone(), |file| file.0.clone());
    let length = replace(destination.clone(), bytes).await?;
    let grant = if let Some(staging) = staging.as_ref() {
        let grant = files::export(destination).await?;
        // The broker owns the offered bytes after export succeeds.
        staging.remove()?;
        Some(grant)
    } else {
        None
    };
    Ok(json!({"name": name, "bytes": length, "grantId": grant}))
}

/// `read_file`: a private diagram by name, or a picked one by grant.
pub async fn read(input: &Value) -> Result<Value, AppError> {
    if input.get("name").is_some() == input.get("grantId").is_some() {
        return Err(error(
            "INVALID_ARGUMENT",
            "provide exactly one of name or grantId",
        ));
    }
    let mut imported = None;
    let name = if let Some(grant) = input["grantId"].as_str() {
        // Exclusive reservation also separates concurrent worker instances.
        imported = Some(ImportFile::reserve()?);
        let destination = imported.as_ref().unwrap().0.clone();
        files::import(grant.into(), destination.clone()).await?;
        destination
    } else {
        scene::filename(&input["name"])?
    };
    // R6 supplies only the installation's data/tmp preopens. The input
    // is one validated filename, never a host path or a directory.
    let file = File::open(format!("data/{name}")).map_err(|_| {
        error(
            "FILE_NOT_FOUND",
            "save a private diagram or pick a file first",
        )
    })?;
    let mut bytes = String::new();
    file.take((scene::MAX_BYTES + 1) as u64)
        .read_to_string(&mut bytes)
        .map_err(|_| error("INVALID_SCENE", "diagram must be readable UTF-8"))?;
    let result = json!({"name": name, "scene": scene::parse(&bytes)?});
    drop(imported);
    Ok(result)
}

/// `pick_file`: ask the person for a diagram; the grants are what they chose.
pub async fn pick() -> Result<Value, AppError> {
    let grants = host::pick_file(host::Pick {
        multiple: false,
        accept: vec![".excalidraw".into(), ".json".into()],
    })
    .await?;
    Ok(json!({"grants": grants}))
}
