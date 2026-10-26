//! Real test App: no WASI adapters, native APIs, or test-only worker commands.
wit_bindgen::generate!({
    path: "../../../app-wit/wit",
    inline: r#"
        package chariox:drill;
        world drill {
            import chariox:app/registry@1.0.0;
            import chariox:app/state@1.0.0;
            import chariox:app/http@1.0.0;
            import chariox:app/events@1.0.0;
            import chariox:app/log@1.0.0;
            import wasi:filesystem/preopens@0.2.0;
            export chariox:app/handlers@1.0.0;
        }
    "#,
    world: "drill",
    generate_all,
});
use chariox::app::{
    app_types::{AppError, InvocationContext, Wake},
    events, http, log, registry, state,
};
use exports::chariox::app::handlers::Guest;
use futures::future::{select, Either};
use serde_json::{json, Value};
use wit_bindgen::{FutureReader, StreamResult};
struct App;
export!(App);
thread_local! { static COUNTER: std::cell::Cell<u32> = const { std::cell::Cell::new(0) }; }
fn fail(code: &str) -> AppError {
    AppError {
        code: code.into(),
        message: code.into(),
        retryable: None,
    }
}
impl Guest for App {
    fn register() -> Result<(), AppError> {
        for name in [
            "echo",
            "shared",
            "wait",
            "ignore",
            "trap",
            "spin",
            "oom",
            "stream",
            "slow_stream",
            "subscribe",
            "upload",
            "early_response",
            "grace",
        ] {
            registry::tool(name);
        }
        Ok(())
    }
    async fn call_tool(
        name: String,
        input: String,
        _context: InvocationContext,
        cancel: FutureReader<String>,
    ) -> Result<String, AppError> {
        if name == "ignore" {
            let _ = state::get("hold".into()).await;
            std::future::pending::<()>().await;
            unreachable!()
        }
        let work = Box::pin(run(name, input));
        match select(
            work,
            std::pin::pin!(std::future::IntoFuture::into_future(cancel)),
        )
        .await
        {
            Either::Left((result, _)) => result,
            Either::Right((_, _)) => Err(fail("CANCELLED")),
        }
    }
    async fn deliver_event(
        _name: String,
        _occurrence_id: String,
        payload: String,
        _context: InvocationContext,
        _cancel: FutureReader<String>,
    ) -> Result<String, AppError> {
        Ok(payload)
    }
    async fn lifecycle(
        _event: String,
        data: Option<String>,
        _context: InvocationContext,
        _cancel: FutureReader<String>,
    ) -> Result<Option<String>, AppError> {
        Ok(data)
    }
    async fn deliver_wake(
        _wake: Wake,
        _overdue: bool,
        _context: InvocationContext,
        _cancel: FutureReader<String>,
    ) -> Result<(), AppError> {
        Ok(())
    }
}
async fn run(name: String, input: String) -> Result<String, AppError> {
    let input: Value = serde_json::from_str(&input).map_err(|_| fail("INVALID_ARGUMENT"))?;
    match name.as_str() {
        "echo" if input["portable"] == "migration-state" => Ok(state::get("migrated".into())
            .await?
            .map_or("null".into(), |record| record.value)),
        "echo" if input["portable"] == "files-write" || input["portable"] == "files-read" => {
            use wasi::filesystem0_2_0::types::{
                DescriptorFlags as D, OpenFlags as O, PathFlags as P,
            };
            let mut values = serde_json::Map::new();
            for (dir, name) in wasi::filesystem0_2_0::preopens::get_directories() {
                let flags = if input["portable"] == "files-write" {
                    O::CREATE
                } else {
                    O::empty()
                };
                let value = match dir.open_at(P::empty(), "marker", flags, D::READ | D::WRITE) {
                    Ok(file) => {
                        if input["portable"] == "files-write" {
                            file.write(b"r6-user-data", 0).unwrap();
                        }
                        Some(String::from_utf8(file.read(100, 0).unwrap().0).unwrap())
                    }
                    Err(_) => None,
                };
                values.insert(name, json!(value));
            }
            Ok(Value::Object(values).to_string())
        }
        "echo" => Ok(input.to_string()),
        "shared" => {
            let count = COUNTER.with(|counter| {
                let next = counter.get() + 1;
                counter.set(next);
                next
            });
            state::get("gate".into()).await?;
            Ok(json!({"count":count}).to_string())
        }
        "grace" => {
            struct Cleanup;
            impl Drop for Cleanup {
                fn drop(&mut self) {
                    log::write(log::Level::Info, "grace-cleanup", "{}");
                }
            }
            let _cleanup = Cleanup;
            state::get("hold".into()).await?;
            Ok("null".into())
        }
        "wait" => {
            state::get("hold".into()).await?;
            Ok("null".into())
        }
        "trap" => core::arch::wasm32::unreachable(),
        "spin" => loop {
            std::hint::spin_loop();
        },
        "oom" => {
            let mut allocations = Vec::new();
            loop {
                let mut bytes = vec![0u8; 1024 * 1024];
                bytes[0] = 1;
                std::hint::black_box(&bytes);
                allocations.push(bytes);
            }
        }
        "slow_stream" => {
            let (_head, _body) = http::fetch(request(), None).await?;
            state::get("hold".into()).await?;
            Ok("null".into())
        }
        "stream" => {
            let (head, mut body) = http::fetch(request(), None).await?;
            let mut total = 0;
            loop {
                let (status, chunk) = body.read(Vec::with_capacity(64 * 1024)).await;
                total += chunk.len();
                if matches!(status, StreamResult::Dropped | StreamResult::Cancelled) {
                    break;
                }
            }
            Ok(json!({"status":head.status,"bytes":total}).to_string())
        }
        "subscribe" => {
            let mut stream = events::subscribe("tick".into()).await?;
            state::get("subscribed".into()).await?;
            if input["slow"] == true {
                state::get("hold".into()).await?;
            }
            let count = usize::from(stream.next().await.is_some());
            Ok(json!({"events":count}).to_string())
        }
        "early_response" => {
            let (mut writer, reader) = wit_stream::new::<u8>();
            let upload = async move {
                let _remaining = writer.write_all(vec![9; 1024 * 1024]).await;
                state::get("upload-stopped".into()).await
            };
            let response = async move {
                let response = http::fetch(request(), Some(reader)).await?;
                drop(response);
                state::get("response-dropped".into()).await
            };
            let (uploaded, dropped) = futures::join!(upload, response);
            uploaded?;
            dropped?;
            Ok("null".into())
        }
        "upload" => {
            let (mut writer, reader) = wit_stream::new::<u8>();
            let upload = async move {
                assert!(writer.write_all(vec![9; 160 * 1024]).await.is_empty());
            };
            let fetch = http::fetch(request(), Some(reader));
            let ((), response) = futures::join!(upload, fetch);
            let _response = response?;
            Ok("null".into())
        }
        _ => Err(fail("TOOL_NOT_FOUND")),
    }
}
fn request() -> http::Request {
    http::Request {
        url: "https://example.com/".into(),
        method: "GET".into(),
        headers: vec![],
        connection_id: None,
        operation_id: None,
    }
}
