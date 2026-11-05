//! Real test App: no WASI adapters, native APIs, or test-only worker commands.
wit_bindgen::generate!({
    path: "../../../app-wit/wit",
    inline: r#"package chariox:wasi-drill;
world drill {
  import chariox:app/registry@1.0.0;
  import chariox:app/state@1.0.0;
  import wasi:http/client@0.3.0;
  import wasi:http/outgoing-handler@0.2.0;
  import wasi:filesystem/preopens@0.3.0;
  import wasi:filesystem/preopens@0.2.0;
  import wasi:random/random@0.3.0;
  import wasi:random/random@0.2.0;
  import wasi:clocks/monotonic-clock@0.3.0;
  import wasi:clocks/monotonic-clock@0.2.0;
  import wasi:cli/stdout@0.3.0;
  import wasi:cli/stderr@0.2.0;
  export chariox:app/handlers@1.0.0;
}
"#,
    world: "drill",
    generate_all,
});
use chariox::app::{
    app_types::{AppError, InvocationContext, Wake},
    registry, state,
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
            "p3-http",
            "p2-http",
            "preopens",
            "runtime",
            "p3-drop",
            "p3-upload-error",
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
async fn run(name: String, _input: String) -> Result<String, AppError> {
    let result = match name.as_str() {
        "p3-http" => p3_http(false).await,
        "p3-drop" => p3_drop().await,
        "p3-upload-error" => p3_upload_error().await,
        "p2-http" => p2_http(),
        "preopens" => preopens().await,
        "runtime" => {
            use wasi::{
                clocks0_2_0 as c2, clocks0_3_0 as c3, random0_2_0 as r2, random0_3_0 as r3,
            };
            let options = wasi::http0_2_0::types::RequestOptions::new();
            assert!(options.set_connect_timeout(Some(1)).is_err());
            options.set_connect_timeout(None).unwrap();
            let options = wasi::http0_3_0::types::RequestOptions::new();
            assert!(matches!(
                options.set_connect_timeout(Some(1)),
                Err(wasi::http0_3_0::types::RequestOptionsError::NotSupported)
            ));
            options.set_connect_timeout(None).unwrap();
            let a = c2::monotonic_clock::now();
            let b = c3::monotonic_clock::now();
            c3::monotonic_clock::wait_for(1_000_000).await;
            assert!(c2::monotonic_clock::now() >= a && c3::monotonic_clock::now() >= b);
            assert_eq!(r2::random::get_random_bytes(8).len(), 8);
            assert_eq!(r3::random::get_random_bytes(8).len(), 8);
            wasi::cli0_2_0::stderr::get_stderr()
                .blocking_write_and_flush(b"wasi-drill")
                .unwrap();
            let (mut writer, reader) = wit_stream::new::<u8>();
            let written = wasi::cli0_3_0::stdout::write_via_stream(reader);
            let upload = async move { writer.write_all(b"wasi-p3".to_vec()).await };
            let (remaining, _) =
                futures::join!(upload, std::future::IntoFuture::into_future(written));
            assert!(remaining.is_empty());
            Ok(json!({"runtime":true}))
        }
        _ => Err("unknown tool".to_owned()),
    }
    .map_err(|_| fail("WASI_DRILL_FAILED"))?;
    Ok(result.to_string())
}
async fn p3_http(drop_body: bool) -> Result<Value, String> {
    use wasi::http0_3_0::{
        client,
        types::{ErrorCode, Fields, Request, Response, Scheme},
    };
    let (writer, trailers) = wit_future::new(|| Ok(None));
    drop(writer);
    let (request, _sent) = Request::new(Fields::new(), None, trailers, None);
    request.set_scheme(Some(&Scheme::Https)).unwrap();
    request.set_authority(Some("example.com")).unwrap();
    request.set_path_with_query(Some("/wasi")).unwrap();
    let response = match client::send(request).await {
        Ok(response) => response,
        Err(error) => return Ok(json!({"denied":matches!(error,ErrorCode::HttpRequestDenied)})),
    };
    if drop_body {
        drop(response);
        state::get("response-dropped".into())
            .await
            .map_err(|e| e.code)?;
        return Ok(Value::Null);
    }
    assert_eq!(response.get_status_code(), 200);
    let (writer, processed) = wit_future::new(|| Ok(()));
    drop(writer);
    let (mut body, _trailers) = Response::consume_body(response, processed);
    let mut bytes = Vec::new();
    loop {
        let (status, chunk) = body.read(Vec::with_capacity(4096)).await;
        bytes.extend(chunk);
        if matches!(status, StreamResult::Dropped | StreamResult::Cancelled) {
            break;
        }
    }
    Ok(json!({"body":String::from_utf8(bytes).unwrap()}))
}
fn p2_http() -> Result<Value, String> {
    use wasi::{
        http0_2_0::{
            outgoing_handler,
            types::{ErrorCode, Fields, OutgoingRequest, Scheme},
        },
        io::streams::StreamError,
    };
    let request = OutgoingRequest::new(Fields::new());
    request.set_scheme(Some(&Scheme::Https)).unwrap();
    request.set_authority(Some("example.com")).unwrap();
    request.set_path_with_query(Some("/wasi")).unwrap();
    let pending = outgoing_handler::handle(request, None).unwrap();
    let response = loop {
        if let Some(result) = pending.get() {
            match result.unwrap() {
                Ok(response) => break response,
                Err(error) => {
                    return Ok(json!({"denied":matches!(error,ErrorCode::HttpRequestDenied)}))
                }
            }
        }
        pending.subscribe().block();
    };
    assert_eq!(response.status(), 200);
    let body = response.consume().unwrap();
    let input = body.stream().unwrap();
    let mut bytes = Vec::new();
    loop {
        match input.blocking_read(4096) {
            Ok(chunk) => bytes.extend(chunk),
            Err(StreamError::Closed) => break,
            Err(_) => return Err("body failed".into()),
        }
    }
    drop(input);
    drop(wasi::http0_2_0::types::IncomingBody::finish(body));
    Ok(json!({"body":String::from_utf8(bytes).unwrap()}))
}
async fn preopens() -> Result<Value, String> {
    use f2::types::{DescriptorFlags as D2, ErrorCode as E2, OpenFlags as O2, PathFlags as P2};
    use f3::types::{DescriptorFlags as D3, ErrorCode as E3, OpenFlags as O3, PathFlags as P3};
    use wasi::{filesystem0_2_0 as f2, filesystem0_3_0 as f3};
    let p2 = f2::preopens::get_directories();
    let p3 = f3::preopens::get_directories();
    assert_eq!(
        p2.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>(),
        ["data", "tmp"]
    );
    assert_eq!(
        p3.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>(),
        ["data", "tmp"]
    );
    for (dir, _) in &p2 {
        let file = dir
            .open_at(P2::empty(), "value", O2::CREATE, D2::READ | D2::WRITE)
            .unwrap();
        file.write(b"scoped", 0).unwrap();
        assert_eq!(file.read(6, 0).unwrap().0, b"scoped");
        for path in ["/etc/passwd", "../outside", "escape"] {
            assert_eq!(
                dir.open_at(P2::SYMLINK_FOLLOW, path, O2::empty(), D2::READ)
                    .unwrap_err(),
                E2::NotPermitted
            );
        }
    }
    for (dir, _) in &p3 {
        let file = dir
            .open_at(P3::empty(), "value".into(), O3::empty(), D3::READ)
            .await
            .unwrap();
        let (mut stream, _done) = file.read_via_stream(0);
        let (_, bytes) = stream.read(Vec::with_capacity(4096)).await;
        assert_eq!(bytes, b"scoped");
        for path in ["/etc/passwd", "../outside", "escape"] {
            assert!(matches!(
                dir.open_at(P3::SYMLINK_FOLLOW, path.into(), O3::empty(), D3::READ)
                    .await,
                Err(E3::NotPermitted)
            ));
        }
    }
    Ok(json!({"preopens":true}))
}

async fn p3_drop() -> Result<Value, String> {
    use wasi::http0_3_0::{
        client,
        types::{Fields, Method, Request, Scheme},
    };
    let (mut writer, reader) = wit_stream::new::<u8>();
    let (trailer_writer, trailers) = wit_future::new(|| Ok(None));
    drop(trailer_writer);
    let (request, _sent) = Request::new(Fields::new(), Some(reader), trailers, None);
    request.set_scheme(Some(&Scheme::Https)).unwrap();
    request.set_authority(Some("example.com")).unwrap();
    request.set_path_with_query(Some("/wasi")).unwrap();
    request.set_method(&Method::Post).unwrap();
    let upload = async move {
        let _remaining = writer.write_all(vec![9; 1024 * 1024]).await;
        state::get("upload-stopped".into()).await
    };
    let response = async move {
        let response = client::send(request)
            .await
            .map_err(|_| fail("HTTP_FAILED"))?;
        drop(response);
        state::get("response-dropped".into()).await
    };
    let (uploaded, dropped) = futures::join!(upload, response);
    uploaded.map_err(|e| e.code)?;
    dropped.map_err(|e| e.code)?;
    Ok(Value::Null)
}

async fn p3_upload_error() -> Result<Value, String> {
    use wasi::http0_3_0::{
        client,
        types::{ErrorCode, Fields, Method, Request, Scheme},
    };
    fn trailers() -> Result<Option<Fields>, ErrorCode> {
        let fields = Fields::new();
        fields.append("x-trailer", b"value").unwrap();
        Ok(Some(fields))
    }
    let (writer, trailers) = wit_future::new(trailers);
    drop(writer);
    let (request, _sent) = Request::new(Fields::new(), None, trailers, None);
    request.set_scheme(Some(&Scheme::Https)).unwrap();
    request.set_authority(Some("example.com")).unwrap();
    request.set_path_with_query(Some("/wasi")).unwrap();
    request.set_method(&Method::Post).unwrap();
    Ok(json!({"denied":matches!(client::send(request).await,Err(ErrorCode::HttpRequestDenied))}))
}
