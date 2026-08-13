use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{AUTHORIZATION, CONTENT_TYPE, ORIGIN};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::net::TcpListener;

use crate::error::DaemonError;
use crate::runtime::router::CommandRouter;

const DEFAULT_PROTOCOL_VERSION: &str = "2025-03-26";
const JSON_RPC_VERSION: &str = "2.0";

type HttpBody = Full<Bytes>;

pub(crate) async fn run_mcp_http_server(router: Arc<CommandRouter>) -> Result<(), DaemonError> {
    let (bind_host, bind_port) = router.runtime_mcp_bind_address();
    let listener = TcpListener::bind((bind_host.as_str(), bind_port))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "bind runtime mcp",
            message: error.to_string(),
        })?;

    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "accept runtime mcp",
                message: error.to_string(),
            })?;
        let router = Arc::clone(&router);
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service = service_fn(move |request| {
                let router = Arc::clone(&router);
                async move { handle_http_request(router, request).await }
            });
            let _ = http1::Builder::new().serve_connection(io, service).await;
        });
    }
}

async fn handle_http_request(
    router: Arc<CommandRouter>,
    request: Request<Incoming>,
) -> Result<Response<HttpBody>, Infallible> {
    let response = match handle_http_request_inner(router, request).await {
        Ok(response) => response,
        Err(error) => text_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("runtime MCP server error: {error}"),
        ),
    };
    Ok(response)
}

async fn handle_http_request_inner(
    router: Arc<CommandRouter>,
    request: Request<Incoming>,
) -> Result<Response<HttpBody>, DaemonError> {
    if let Some(origin) = request
        .headers()
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        if !origin.starts_with("http://127.0.0.1")
            && !origin.starts_with("http://localhost")
            && !origin.starts_with("https://127.0.0.1")
            && !origin.starts_with("https://localhost")
        {
            return Ok(text_response(
                StatusCode::FORBIDDEN,
                "invalid origin".to_string(),
            ));
        }
    }

    if let Some(name) = request.uri().path().strip_prefix("/mcp/proxy/") {
        return handle_proxy_json_rpc_request(router, name.to_string(), request).await;
    }

    if request.uri().path() != "/mcp" {
        return Ok(text_response(
            StatusCode::NOT_FOUND,
            "not found".to_string(),
        ));
    }

    match *request.method() {
        Method::GET => Ok(empty_response(StatusCode::METHOD_NOT_ALLOWED)),
        Method::POST => handle_json_rpc_request(router, request).await,
        Method::DELETE => Ok(empty_response(StatusCode::METHOD_NOT_ALLOWED)),
        _ => Ok(empty_response(StatusCode::METHOD_NOT_ALLOWED)),
    }
}

async fn handle_proxy_json_rpc_request(
    router: Arc<CommandRouter>,
    name: String,
    request: Request<Incoming>,
) -> Result<Response<HttpBody>, DaemonError> {
    if request.method() != Method::POST {
        return Ok(empty_response(StatusCode::METHOD_NOT_ALLOWED));
    }
    let auth_token =
        parse_bearer_token(request.headers()).ok_or_else(|| DaemonError::LocalTransport {
            operation: "mcp_proxy_auth",
            message: "missing or invalid bearer token".to_string(),
        });
    let auth_token = match auth_token {
        Ok(token) => token,
        Err(_) => {
            return Ok(text_response(
                StatusCode::UNAUTHORIZED,
                "unauthorized".to_string(),
            ))
        }
    };
    let body = request
        .into_body()
        .collect()
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "mcp_proxy_read_body",
            message: error.to_string(),
        })?
        .to_bytes();
    let payload =
        serde_json::from_slice::<Value>(&body).map_err(|error| DaemonError::LocalTransport {
            operation: "mcp_proxy_parse_json",
            message: error.to_string(),
        })?;
    let id = payload.get("id").cloned();
    if id.is_none()
        && payload
            .get("method")
            .and_then(Value::as_str)
            .is_some_and(|method| method.starts_with("notifications/"))
    {
        return Ok(empty_response(StatusCode::ACCEPTED));
    }
    match router
        .dispatch_authenticated_mcp_proxy_call(&auth_token, &name, payload)
        .await
    {
        Ok(response) => Ok(json_response(StatusCode::OK, response)),
        Err(error) => Ok(json_rpc_error_response(id, -32000, &error.to_string())),
    }
}

async fn handle_json_rpc_request(
    router: Arc<CommandRouter>,
    request: Request<Incoming>,
) -> Result<Response<HttpBody>, DaemonError> {
    let auth_token =
        parse_bearer_token(request.headers()).ok_or_else(|| DaemonError::LocalTransport {
            operation: "runtime_mcp_auth",
            message: "missing or invalid bearer token".to_string(),
        });
    let auth_token = match auth_token {
        Ok(token) => token,
        Err(_) => {
            return Ok(text_response(
                StatusCode::UNAUTHORIZED,
                "unauthorized".to_string(),
            ))
        }
    };
    let body = request
        .into_body()
        .collect()
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "runtime_mcp_read_body",
            message: error.to_string(),
        })?
        .to_bytes();
    let payload =
        serde_json::from_slice::<Value>(&body).map_err(|error| DaemonError::LocalTransport {
            operation: "runtime_mcp_parse_json",
            message: error.to_string(),
        })?;
    handle_json_rpc_value(router, &auth_token, payload).await
}

async fn handle_json_rpc_value(
    router: Arc<CommandRouter>,
    auth_token: &str,
    payload: Value,
) -> Result<Response<HttpBody>, DaemonError> {
    let Some(method) = payload.get("method").and_then(Value::as_str) else {
        return Ok(json_rpc_error_response(
            payload.get("id").cloned(),
            -32600,
            "invalid request",
        ));
    };
    let id = payload.get("id").cloned();
    if id.is_none() {
        return Ok(empty_response(StatusCode::ACCEPTED));
    }

    match method {
        "initialize" => {
            let protocol_version = payload
                .get("params")
                .and_then(|params| params.get("protocolVersion"))
                .and_then(Value::as_str)
                .unwrap_or(DEFAULT_PROTOCOL_VERSION);
            Ok(json_response(
                StatusCode::OK,
                serde_json::json!({
                    "jsonrpc": JSON_RPC_VERSION,
                    "id": id,
                    "result": {
                        "protocolVersion": protocol_version,
                        "capabilities": {
                            "tools": {
                                "listChanged": false
                            },
                            "resources": {
                                "subscribe": false,
                                "listChanged": false
                            },
                            "prompts": {
                                "listChanged": false
                            }
                        },
                        "serverInfo": {
                            "name": "chariox-runtime",
                            "version": env!("CARGO_PKG_VERSION"),
                        }
                    }
                }),
            ))
        }
        "tools/list" => Ok(json_response(
            StatusCode::OK,
            serde_json::json!({
                "jsonrpc": JSON_RPC_VERSION,
                "id": id,
                "result": {
                    "tools": router
                        .runtime_tool_specs_for_auth_token(auth_token)
                        .into_iter()
                        .map(|tool| serde_json::json!({
                            "name": tool.name,
                            "description": tool.description,
                            "inputSchema": tool.input_schema,
                        }))
                        .collect::<Vec<_>>()
                }
            }),
        )),
        "resources/list" => Ok(json_response(
            StatusCode::OK,
            serde_json::json!({
                "jsonrpc": JSON_RPC_VERSION,
                "id": id,
                "result": {
                    "resources": []
                }
            }),
        )),
        "resources/templates/list" => Ok(json_response(
            StatusCode::OK,
            serde_json::json!({
                "jsonrpc": JSON_RPC_VERSION,
                "id": id,
                "result": {
                    "resourceTemplates": []
                }
            }),
        )),
        "prompts/list" => Ok(json_response(
            StatusCode::OK,
            serde_json::json!({
                "jsonrpc": JSON_RPC_VERSION,
                "id": id,
                "result": {
                    "prompts": []
                }
            }),
        )),
        "tools/call" => {
            let params = payload.get("params").cloned().unwrap_or(Value::Null);
            let Some(tool_name) = params.get("name").and_then(Value::as_str) else {
                return Ok(json_rpc_error_response(id, -32602, "missing tool name"));
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let result = router
                .dispatch_authenticated_runtime_tool_call(auth_token, tool_name, arguments)
                .await;
            match result {
                Ok(result) => Ok(json_response(
                    StatusCode::OK,
                    serde_json::json!({
                        "jsonrpc": JSON_RPC_VERSION,
                        "id": id,
                        "result": {
                            "content": [{
                                "type": "text",
                                "text": result.payload.to_string(),
                            }],
                            "structuredContent": result.payload,
                            "isError": !result.ok,
                        }
                    }),
                )),
                Err(error) => Ok(json_rpc_error_response(id, -32000, &error.to_string())),
            }
        }
        _ => Ok(json_rpc_error_response(id, -32601, "method not found")),
    }
}

fn parse_bearer_token(headers: &hyper::HeaderMap) -> Option<String> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?.trim();
    value
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

fn empty_response(status: StatusCode) -> Response<HttpBody> {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::new()))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
}

fn text_response(status: StatusCode, body: String) -> Response<HttpBody> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Full::new(Bytes::from(body)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
}

fn json_response(status: StatusCode, value: Value) -> Response<HttpBody> {
    let body = serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
}

fn json_rpc_error_response(id: Option<Value>, code: i64, message: &str) -> Response<HttpBody> {
    json_response(
        StatusCode::OK,
        serde_json::json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": id.unwrap_or(Value::Null),
            "error": {
                "code": code,
                "message": message,
            }
        }),
    )
}

#[cfg(test)]
mod tests;
