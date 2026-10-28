use chariox_app_sdk::{
    app_types::{AppError, InvocationContext, Wake},
    cancellable, error, files, host, registry, state, FutureReader, Guest,
};
use serde_json::{json, Value};
use std::{fs::File, io::Read};

mod scene;

struct Excalidraw;
chariox_app_sdk::bindings::export!(Excalidraw with_types_in chariox_app_sdk::bindings);

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
        let _ = std::fs::remove_file(format!("data/{}", self.0));
        let _ = std::fs::remove_dir(format!("data/{}", self.0.rsplit_once('/').unwrap().0));
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

async fn call(name: &str, input: &str) -> Result<Value, AppError> {
    let input: Value =
        serde_json::from_str(input).map_err(|e| error("INVALID_ARGUMENT", e.to_string()))?;
    match name {
        "load_scene" => {
            let record = state::get("scene".into()).await?;
            Ok(match record {
                Some(record) => json!({
                    "scene": scene::parse(&record.value)?, "version": record.version.to_string()
                }),
                None => json!({"scene": null, "version": null}),
            })
        }
        "save_scene" => {
            let scene = scene::parse(&input["scene"].to_string())?;
            let version =
                match &input["version"] {
                    Value::Null => None,
                    Value::String(v) => Some(v.parse::<u64>().map_err(|_| {
                        error("INVALID_ARGUMENT", "version must be a decimal string")
                    })?),
                    _ => {
                        return Err(error(
                            "INVALID_ARGUMENT",
                            "version must be a string or null",
                        ))
                    }
                };
            let committed = state::transact(state::Transaction {
                schema_version: 0,
                checks: vec![state::Check {
                    key: "scene".into(),
                    version,
                }],
                writes: vec![state::Write::Put(("scene".into(), scene.to_string()))],
                occurrences: vec![],
                wakes: vec![],
            })
            .await?;
            Ok(json!({"version": committed.revision.to_string()}))
        }
        "write_file" => {
            let name = scene::filename(&input["name"])?;
            let bytes = scene::file_bytes(&input)?;
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
        "read_file" => {
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
        "pick_file" => {
            let grants = host::pick_file(host::Pick {
                multiple: false,
                accept: vec![".excalidraw".into(), ".json".into()],
            })
            .await?;
            Ok(json!({"grants": grants}))
        }
        _ => Err(error("TOOL_NOT_FOUND", name)),
    }
}

impl Guest for Excalidraw {
    fn register() -> Result<(), AppError> {
        for name in [
            "load_scene",
            "save_scene",
            "write_file",
            "read_file",
            "pick_file",
        ] {
            registry::tool(name);
        }
        Ok(())
    }
    async fn call_tool(
        name: String,
        input: String,
        _: InvocationContext,
        cancel: FutureReader<String>,
    ) -> Result<String, AppError> {
        cancellable(cancel, async { Ok(call(&name, &input).await?.to_string()) }).await
    }
    async fn deliver_event(
        _: String,
        _: String,
        _: String,
        _: InvocationContext,
        _: FutureReader<String>,
    ) -> Result<String, AppError> {
        Err(error("EVENT_NOT_FOUND", "no incoming events declared"))
    }
    async fn lifecycle(
        _: String,
        _: Option<String>,
        _: InvocationContext,
        _: FutureReader<String>,
    ) -> Result<Option<String>, AppError> {
        Ok(None)
    }
    async fn deliver_wake(
        _: Wake,
        _: bool,
        _: InvocationContext,
        _: FutureReader<String>,
    ) -> Result<(), AppError> {
        Err(error("WAKE_NOT_FOUND", "no wakes declared"))
    }
}
