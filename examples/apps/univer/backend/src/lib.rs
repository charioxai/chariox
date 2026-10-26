use chariox_app_sdk::{
    app_types::{AppError, InvocationContext, Wake},
    cancellable, error, files, host, registry, state, FutureReader, Guest,
};
use serde_json::{json, Value};
use std::{fs::File, io::Read};

mod document;
mod save;
mod snapshot;

struct Sheets;
chariox_app_sdk::bindings::export!(Sheets with_types_in chariox_app_sdk::bindings);

fn invalid(message: impl Into<String>) -> AppError {
    error("INVALID_ARGUMENT", message)
}

async fn write_bytes(name: String, bytes: Vec<u8>) -> Result<u64, AppError> {
    let expected = bytes.len() as u64;
    let (mut sink, source) = chariox_app_sdk::bindings::wit_stream::new();
    let send = async move {
        let unwritten = sink.write_all(bytes).await;
        // The host commits only after EOF. Keeping the writer through join!
        // deadlocks the commit against the writer's own eventual drop.
        drop(sink);
        unwritten
    };
    let (commit, unwritten) = futures::join!(files::atomic_replace(name, source), send);
    let written = commit?;
    if !unwritten.is_empty() || written != expected {
        return Err(error(
            "FILE_WRITE_FAILED",
            "file was not fully written; retry the save or export",
        ));
    }
    Ok(written)
}

fn read_document(name: &str) -> Result<String, AppError> {
    read_document_at(std::path::Path::new("data"), name)
}

fn read_document_at(data: &std::path::Path, name: &str) -> Result<String, AppError> {
    let file = File::open(data.join(name)).map_err(|_| {
        error(
            "FILE_NOT_FOUND",
            "pick a file or export it to App data first",
        )
    })?;
    let mut text = String::new();
    file.take((document::MAX_BYTES + 1) as u64)
        .read_to_string(&mut text)
        .map_err(|_| invalid("file must be readable UTF-8"))?;
    document::file(name, &text).map_err(invalid)?;
    Ok(text)
}

fn text_chunk(text: &str, input: &Value) -> Result<Value, AppError> {
    let offset = input["offset"].as_u64().unwrap_or(0) as usize;
    if offset > text.len() || !text.is_char_boundary(offset) {
        return Err(invalid("invalid file offset"));
    }
    let mut end = (offset + 64 * 1024).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Ok(json!({"text":&text[offset..end],"next":end,"done":end == text.len()}))
}

async fn stage_export(name: &str, input: &Value) -> Result<u64, AppError> {
    use std::io::Write;
    let text = input["text"]
        .as_str()
        .ok_or_else(|| invalid("text is required"))?;
    let offset = input["offset"]
        .as_u64()
        .ok_or_else(|| invalid("offset is required"))?;
    let limit = if name.ends_with(".csv") {
        document::MAX_CSV_BYTES
    } else {
        document::MAX_BYTES
    };
    if text.len() > 64 * 1024 || offset > limit as u64 || text.len() > limit - offset as usize {
        return Err(invalid(if name.ends_with(".csv") {
            "CSV exceeds 192 KiB (196608 bytes)"
        } else {
            "workbook file exceeds 512 KiB (524288 bytes), including file metadata"
        }));
    }
    let staging = format!("data/.{name}.pending");
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    if offset == 0 {
        options.create(true).truncate(true);
    } else {
        options.append(true);
    }
    let mut file = options.open(&staging).map_err(|_| {
        error(
            "FILE_WRITE_FAILED",
            "file upload could not open; retry from the beginning",
        )
    })?;
    if file
        .metadata()
        .map_err(|_| invalid("file upload metadata is unavailable"))?
        .len()
        != offset
    {
        return Err(invalid(
            "file upload offset changed; retry from the beginning",
        ));
    }
    file.write_all(text.as_bytes()).map_err(|_| {
        error(
            "FILE_WRITE_FAILED",
            "file upload failed; check App data space and retry",
        )
    })?;
    let bytes = offset + text.len() as u64;
    if input["final"] == true {
        file.sync_all()
            .map_err(|_| error("FILE_WRITE_FAILED", "file upload could not sync; retry"))?;
        drop(file);
        let full =
            std::fs::read_to_string(&staging).map_err(|_| invalid("file upload is unreadable"))?;
        document::file(name, &full).map_err(invalid)?;
        write_bytes(name.into(), full.into_bytes()).await?;
        std::fs::remove_file(staging).map_err(|_| {
            error(
                "FILE_WRITE_FAILED",
                "file uploaded, but staging cleanup failed; retry",
            )
        })?;
    }
    Ok(bytes)
}

async fn dispatch(name: &str, input: Value) -> Result<Value, AppError> {
    match name {
        "load_workbook" => match state::get("workbook".into()).await? {
            None => {
                save::sweep(std::path::Path::new("data"), None);
                Ok(json!({"text":null,"version":null}))
            }
            Some(record) => {
                let book: Value = serde_json::from_str(&record.value).map_err(|_| {
                    error(
                        "INVALID_STATE",
                        "saved workbook is invalid JSON; open an exported copy",
                    )
                })?;
                save::sweep(std::path::Path::new("data"), Some(&book));
                let book = snapshot::read(book)?;
                let mut result = text_chunk(&book.to_string(), &input)?;
                result["version"] = json!(record.version.to_string());
                Ok(result)
            }
        },
        "save_workbook" => {
            let name = document::filename(&input["name"]).map_err(invalid)?;
            if !name.ends_with(".univer.json") {
                return Err(invalid("save a .univer.json workbook"));
            }
            let revision = save::workbook(
                std::path::Path::new("data"),
                &name,
                &input["version"],
                async || chariox_app_sdk::get_json("workbook").await,
                snapshot::write,
                async |pointer, version| {
                    let tx = state::Transaction {
                        schema_version: 0,
                        checks: vec![state::Check {
                            key: "workbook".into(),
                            version,
                        }],
                        writes: vec![state::Write::Put(("workbook".into(), pointer.to_string()))],
                        occurrences: vec![],
                        wakes: vec![],
                    };
                    Ok(state::transact(tx).await?.revision)
                },
            )
            .await?;
            Ok(json!({"version":revision.to_string()}))
        }
        "write_file" => {
            let name = document::filename(&input["name"]).map_err(invalid)?;
            let written = stage_export(&name, &input).await?;
            let grant = if input["offer"] == true && input["final"] == true {
                Some(files::export(name.clone()).await?)
            } else {
                None
            };
            Ok(json!({"name":name,"bytes":written,"grantId":grant}))
        }
        "read_file" => {
            let name = document::filename(&input["name"]).map_err(invalid)?;
            if let Some(grant) = input["grantId"].as_str() {
                files::import(grant.into(), name.clone()).await?;
            }
            let text = read_document(&name)?;
            text_chunk(&text, &input)
        }

        "pick_file" => {
            let extension = match input["format"].as_str() {
                Some("csv") => ".csv",
                Some("workbook") => ".univer.json",
                _ => return Err(invalid("format must be csv or workbook")),
            };
            let grants = host::pick_file(host::Pick {
                multiple: false,
                accept: vec![extension.into()],
            })
            .await?;
            Ok(json!({"grants":grants}))
        }
        _ => Err(error("TOOL_NOT_FOUND", "unknown Univer backend tool")),
    }
}

impl Guest for Sheets {
    fn register() -> Result<(), AppError> {
        for tool in [
            "load_workbook",
            "save_workbook",
            "write_file",
            "read_file",
            "pick_file",
        ] {
            registry::tool(tool);
        }
        Ok(())
    }

    async fn call_tool(
        name: String,
        input: String,
        _: InvocationContext,
        cancel: FutureReader<String>,
    ) -> Result<String, AppError> {
        cancellable(cancel, async {
            let value = serde_json::from_str(&input).map_err(|_| invalid("input must be JSON"))?;
            Ok(dispatch(&name, value).await?.to_string())
        })
        .await
    }

    async fn deliver_event(
        _: String,
        _: String,
        _: String,
        _: InvocationContext,
        _: FutureReader<String>,
    ) -> Result<String, AppError> {
        Err(error(
            "EVENT_NOT_FOUND",
            "Univer declares no incoming events",
        ))
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
        Err(error("WAKE_NOT_FOUND", "Univer declares no wakes"))
    }
}
