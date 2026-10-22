//! MP-08/MP-10/MP-11 research: Tectonic on the existing async App contract.
wit_bindgen::generate!({ world: "app", path: "/root/work/agent-texspike/spikes/wasm/wit", generate_all });
use chariox::app::{http, registry};
use chariox::app::types::{AppError, InvocationContext};
use exports::chariox::app::handlers::Guest;
use serde_json::{json, Value};
use std::io::Write;
use wit_bindgen::{FutureReader, StreamResult};
mod driver;
struct App;
export!(App);
fn fail(message: impl ToString) -> AppError {
    AppError { code: "TEX_ERROR".into(), message: message.to_string(), retryable: Some(false) }
}
fn checked_name(name: &str) -> Result<&str, AppError> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name == "." || name == ".." { Err(fail("flat project filenames required in this spike")) } else { Ok(name) }
}
impl Guest for App {
    fn register() -> Result<(), AppError> {
        registry::tool("compile");
        registry::tool("prepare_resources");
        registry::tool("filesystem_probe");
        Ok(())
    }
    async fn call_tool(name: String, input: String, _: InvocationContext, _: FutureReader<String>) -> Result<String, AppError> {
        if name == "filesystem_probe" {
            return std::fs::write("data/probe", b"ok").map(|_| "true".into()).map_err(fail);
        }
        if name != "compile" && name != "prepare_resources" { return Err(fail("unknown tool")); }
        let v: Value = serde_json::from_str(&input).map_err(fail)?;
        for dir in ["data/input", "data/output", "data/bundle", "data/cache"] { std::fs::create_dir_all(dir).map_err(fail)?; }
        let mut fetched = 0;
        for resource in v["resources"].as_array().ok_or_else(|| fail("resources required"))? {
            let name = checked_name(resource.as_str().ok_or_else(|| fail("resource name"))?)?;
            let dest = format!("data/bundle/{name}");
            if std::path::Path::new(&dest).exists() { continue; }
            let req = http::Request { url: format!("https://spike-origin.example/{name}"), method: "GET".into(), headers: vec![], body: None, connection_id: None, operation_id: None };
            let (head, mut body) = http::fetch(req).await?;
            if head.status != 200 { return Err(fail(format!("bundle fetch {name}: {}", head.status))); }
            let temporary = format!("{dest}.part");
            let mut file = std::fs::File::create(&temporary).map_err(fail)?;
            loop {
                let (result, chunk) = body.read(Vec::with_capacity(64 * 1024)).await;
                match result {
                    StreamResult::Complete(_) => { file.write_all(&chunk).map_err(fail)?; fetched += chunk.len(); }
                    StreamResult::Dropped => break,
                    StreamResult::Cancelled => return Err(fail("bundle stream cancelled")),
                }
            }
            drop(file);
            std::fs::rename(temporary, dest).map_err(fail)?;
        }
        if name == "prepare_resources" { return Ok(json!({"fetchedBytes": fetched}).to_string()); }
        for (name, contents) in v["files"].as_object().ok_or_else(|| fail("files required"))? {
            let name = checked_name(name)?;
            let bytes: Vec<u8> = serde_json::from_value(contents.clone()).map_err(fail)?;
            std::fs::write(format!("data/input/{name}"), bytes).map_err(fail)?;
        }
        if !std::path::Path::new("data/cache/latex.fmt").exists() {
            driver::generate_format("data/bundle").map_err(fail)?;
        }
        let name = checked_name(v["main"].as_str().ok_or_else(|| fail("main required"))?)?;
        driver::compile(&format!("/data/input/{name}"), "data/output", "data/bundle").map_err(fail)?;
        let base = name.trim_end_matches(".tex");
        let pdf = std::fs::read(format!("data/output/{base}.pdf")).map_err(fail)?;
        for ext in ["aux", "toc", "lof", "lot", "out", "bbl"] {
            let from = format!("data/output/{base}.{ext}");
            if let Ok(data) = std::fs::read(&from) { std::fs::write(format!("data/input/{base}.{ext}"), data).map_err(fail)?; }
        }
        Ok(json!({"pdfBytes": pdf.len(), "fetchedBytes": fetched, "pdfHeader": String::from_utf8_lossy(&pdf[..pdf.len().min(8)])}).to_string())
    }
}
