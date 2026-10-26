use chariox_app_sdk::{
    app_types::{AppError, InvocationContext, Wake},
    cancellable, error, files, host, registry, state, FutureReader, Guest,
};
use serde_json::{json, Value};
use std::{fs::File, io::Read};

mod document;

struct Sheets;
chariox_app_sdk::bindings::export!(Sheets with_types_in chariox_app_sdk::bindings);

fn invalid(message: impl Into<String>) -> AppError {
    error("INVALID_ARGUMENT", message)
}

async fn write_document(name: String, text: String) -> Result<u64, AppError> {
    let bytes = text.into_bytes();
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
            "file was not fully written; retry the export",
        ));
    }
    Ok(written)
}

async fn dispatch(name: &str, input: Value) -> Result<Value, AppError> {
    match name {
        "load_workbook" => match state::get("workbook".into()).await? {
            None => Ok(json!({"workbook":null,"version":null})),
            Some(record) => {
                let book: Value = serde_json::from_str(&record.value).map_err(|_| {
                    error(
                        "INVALID_STATE",
                        "saved workbook is invalid JSON; open an exported copy",
                    )
                })?;
                document::workbook(&book).map_err(|e| error("INVALID_STATE", e))?;
                Ok(json!({"workbook":book,"version":record.version.to_string()}))
            }
        },
        "save_workbook" => {
            let book = &input["workbook"];
            document::workbook(book).map_err(invalid)?;
            let version = match &input["version"] {
                Value::Null => None,
                Value::String(text) => Some(
                    text.parse::<u64>()
                        .map_err(|_| invalid("version must be a decimal string or null"))?,
                ),
                _ => return Err(invalid("version must be a decimal string or null")),
            };
            let tx = state::Transaction {
                schema_version: 0,
                checks: vec![state::Check {
                    key: "workbook".into(),
                    version,
                }],
                writes: vec![state::Write::Put(("workbook".into(), book.to_string()))],
                occurrences: vec![],
                wakes: vec![],
            };
            let receipt = state::transact(tx).await?;
            Ok(json!({"version":receipt.revision.to_string()}))
        }
        "write_file" => {
            let name = document::filename(&input["name"]).map_err(invalid)?;
            let text = input["text"]
                .as_str()
                .ok_or_else(|| invalid("text is required"))?;
            document::file(&name, text).map_err(invalid)?;
            let written = write_document(name.clone(), text.into()).await?;
            let grant = if input["offer"] == true {
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
            // The worker grants only data/tmp preopens. A validated basename is
            // read in data after the kernel has imported a user-selected grant.
            let file = File::open(format!("data/{name}")).map_err(|_| {
                error(
                    "FILE_NOT_FOUND",
                    "pick a file or export it to App data first",
                )
            })?;
            let mut text = String::new();
            file.take((document::MAX_BYTES + 129) as u64)
                .read_to_string(&mut text)
                .map_err(|_| invalid("file must be readable UTF-8"))?;
            document::file(&name, &text).map_err(invalid)?;
            Ok(json!({"name":name,"text":text}))
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
