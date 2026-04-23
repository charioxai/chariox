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
                            "name": "arroba-runtime",
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
                    "tools": crate::transport::runtime_tools::managed_io_runtime_tool_specs()
                        .into_iter()
                        .chain(crate::transport::runtime_tools::capability_runtime_tool_specs())
                        .chain(crate::transport::runtime_tools::credential_runtime_tool_specs())
                        .chain(crate::transport::runtime_tools::workflow_runtime_tool_specs())
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
mod tests {
    use std::sync::Arc;

    use http_body_util::BodyExt;
    use hyper::StatusCode;
    use serde_json::Value;
    use tokio::sync::Mutex;

    use crate::agent::CreateAgentRequest;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::runtime::router::CommandRouter;
    use crate::session::CreateSessionRequest;
    use crate::{DaemonApp, DaemonConfig};

    use super::handle_json_rpc_value;

    #[tokio::test]
    async fn mcp_initialize_and_tools_list_return_runtime_tools() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = Arc::new(CommandRouter::with_interactive_capacity(app, 8));

        let initialize = handle_json_rpc_value(
            router.clone(),
            "unused-token",
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26"
                }
            }),
        )
        .await
        .expect("initialize should succeed");
        assert_eq!(initialize.status(), StatusCode::OK);
        let initialize_body = initialize
            .into_body()
            .collect()
            .await
            .expect("initialize body should collect")
            .to_bytes();
        let initialize_value: Value =
            serde_json::from_slice(&initialize_body).expect("initialize body should be json");
        assert_eq!(
            initialize_value["result"]["serverInfo"]["name"],
            "arroba-runtime"
        );

        let tools_list = handle_json_rpc_value(
            router.clone(),
            "unused-token",
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": {}
            }),
        )
        .await
        .expect("tools/list should succeed");
        assert_eq!(tools_list.status(), StatusCode::OK);
        let tools_body = tools_list
            .into_body()
            .collect()
            .await
            .expect("tools list body should collect")
            .to_bytes();
        let tools_value: Value =
            serde_json::from_slice(&tools_body).expect("tools list body should be json");
        let tools = tools_value["result"]["tools"]
            .as_array()
            .expect("tools should be an array");
        assert!(tools.iter().any(|tool| tool["name"] == "ack_workflow_turn"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "validate_workflow_output"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "workflow_console_read"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "workflow_console_write"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "workflow_console_clear"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "arroba.read_artifact"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "arroba.edit_artifact"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "arroba.apply_patch"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "arroba.delete_artifact"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "arroba.move_artifact"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "arroba.write_artifact"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "arroba.list_capabilities"));
        assert!(tools.iter().any(|tool| tool["name"] == "list_capabilities"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "arroba.request_capability"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "request_capability"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "arroba.list_credential_handles"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "list_credential_handles"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "arroba.http_request_with_credential"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "http_request_with_credential"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "arroba.send_secret_to_terminal"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "send_secret_to_terminal"));
    }

    #[tokio::test]
    async fn mcp_resource_and_prompt_discovery_return_empty_lists() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = Arc::new(CommandRouter::with_interactive_capacity(app, 8));

        for (id, method, result_key) in [
            (1, "resources/list", "resources"),
            (2, "resources/templates/list", "resourceTemplates"),
            (3, "prompts/list", "prompts"),
        ] {
            let response = handle_json_rpc_value(
                router.clone(),
                "unused-token",
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": method,
                    "params": {}
                }),
            )
            .await
            .expect("discovery request should succeed");
            assert_eq!(response.status(), StatusCode::OK);
            let body = response
                .into_body()
                .collect()
                .await
                .expect("discovery body should collect")
                .to_bytes();
            let value: Value = serde_json::from_slice(&body).expect("discovery body json");
            assert_eq!(value["id"], id);
            assert_eq!(
                value["result"][result_key]
                    .as_array()
                    .expect("discovery result should be an array")
                    .len(),
                0
            );
        }
    }

    #[tokio::test]
    async fn mcp_http_tools_call_acknowledges_active_workflow_turn() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should exist");
        crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "client-a",
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("attachment should attach");
        let agent_id = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("agent-a")
                    .with_model("test-model")
                    .with_worktree("worktree-1"),
            )
            .expect("agent should spawn")
            .id()
            .to_string();
        let workflow_id = app
            .sessions_mut()
            .create_workflow(session.id(), Some("wf".to_string()))
            .expect("workflow should exist")
            .id()
            .to_string();
        let node_id = app
            .sessions_mut()
            .add_workflow_node(session.id(), &workflow_id, &agent_id)
            .expect("node should be added")
            .id()
            .to_string();
        app.sessions_mut()
            .create_workflow_endpoint(
                session.id(),
                &workflow_id,
                &node_id,
                Some("entry".to_string()),
            )
            .expect("endpoint should exist");
        let (workflow_run, _, _) = app
            .invoke_workflow_endpoint_and_schedule(
                session.id(),
                &workflow_id,
                "entry",
                Some("start".to_string()),
            )
            .expect("workflow should invoke");
        let node_run = workflow_run
            .node_runs()
            .first()
            .expect("node run should exist");
        let envelope = node_run.turn_envelope().expect("envelope should exist");
        let auth_token = app
            .providers()
            .get_run_for_agent(session.id(), &agent_id)
            .expect("provider run should exist")
            .runtime_mcp_auth_token()
            .expect("mcp auth token should exist")
            .to_string();

        let app = Arc::new(Mutex::new(app));
        let router = Arc::new(CommandRouter::with_interactive_capacity(app, 8));
        let response = handle_json_rpc_value(
            router.clone(),
            &auth_token,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "ack_workflow_turn",
                    "arguments": {
                        "delivery_token": envelope.delivery_token(),
                    }
                }
            }),
        )
        .await
        .expect("mcp request should succeed");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let value: Value = serde_json::from_slice(&body).expect("body should be json");
        assert_eq!(
            value["result"]["structuredContent"]["state"],
            "acknowledged"
        );
    }

    #[tokio::test]
    async fn mcp_http_tools_call_reads_and_edits_managed_artifact() {
        let root = std::env::temp_dir().join(format!(
            "arroba-managed-io-mcp-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before unix epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("test root should be created");
        std::fs::write(root.join("notes.txt"), "alpha\nbeta\n").expect("file should be written");

        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let worktree = root.to_string_lossy().to_string();
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", &worktree))
            .expect("session should exist");
        let agent_id = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("agent-a")
                    .with_model("test-model")
                    .with_worktree(&worktree),
            )
            .expect("agent should spawn")
            .id()
            .to_string();
        let workflow_id = app
            .sessions_mut()
            .create_workflow(session.id(), Some("wf".to_string()))
            .expect("workflow should exist")
            .id()
            .to_string();
        let node_id = app
            .sessions_mut()
            .add_workflow_node(session.id(), &workflow_id, &agent_id)
            .expect("node should be added")
            .id()
            .to_string();
        app.sessions_mut()
            .create_workflow_endpoint(
                session.id(),
                &workflow_id,
                &node_id,
                Some("entry".to_string()),
            )
            .expect("endpoint should exist");
        app.invoke_workflow_endpoint_and_schedule(
            session.id(),
            &workflow_id,
            "entry",
            Some("start".to_string()),
        )
        .expect("workflow should invoke");
        let auth_token = app
            .providers()
            .get_run_for_agent(session.id(), &agent_id)
            .expect("provider run should exist")
            .runtime_mcp_auth_token()
            .expect("mcp auth token should exist")
            .to_string();

        let app = Arc::new(Mutex::new(app));
        let router = Arc::new(CommandRouter::with_interactive_capacity(app, 8));
        let read_response = handle_json_rpc_value(
            router.clone(),
            &auth_token,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "arroba.read_artifact",
                    "arguments": {
                        "path": "notes.txt"
                    }
                }
            }),
        )
        .await
        .expect("read request should succeed");
        assert_eq!(read_response.status(), StatusCode::OK);
        let read_body = read_response
            .into_body()
            .collect()
            .await
            .expect("read body should collect")
            .to_bytes();
        let read_value: Value = serde_json::from_slice(&read_body).expect("read body json");
        assert_eq!(
            read_value["result"]["structuredContent"]["content_text"],
            "alpha\nbeta\n"
        );
        assert_eq!(
            read_value["result"]["structuredContent"]["workspace"]["identity_changed"],
            false
        );
        let snapshot_id = read_value["result"]["structuredContent"]["snapshot_id"]
            .as_str()
            .expect("snapshot id should be present")
            .to_string();
        let edit_response = handle_json_rpc_value(
            router.clone(),
            &auth_token,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "arroba.edit_artifact",
                    "arguments": {
                        "path": "notes.txt",
                        "snapshot_id": snapshot_id,
                        "old_text": "beta",
                        "new_text": "gamma"
                    }
                }
            }),
        )
        .await
        .expect("edit request should succeed");
        assert_eq!(edit_response.status(), StatusCode::OK);
        let edit_body = edit_response
            .into_body()
            .collect()
            .await
            .expect("edit body should collect")
            .to_bytes();
        let edit_value: Value = serde_json::from_slice(&edit_body).expect("edit body json");
        assert_eq!(edit_value["result"]["structuredContent"]["applied"], true);
        assert_eq!(
            edit_value["result"]["structuredContent"]["workspace"]["identity_changed"],
            false
        );
        assert_eq!(
            edit_value["result"]["structuredContent"]["change"]["kind"],
            "update"
        );
        assert!(edit_value["result"]["structuredContent"]["change"]["diff"]
            .as_str()
            .expect("edit diff should be present")
            .contains("-beta"));
        assert!(edit_value["result"]["structuredContent"]["change"]["diff"]
            .as_str()
            .expect("edit diff should be present")
            .contains("+gamma"));
        assert_eq!(
            std::fs::read_to_string(root.join("notes.txt")).expect("file should be readable"),
            "alpha\ngamma\n"
        );

        let patch_response = handle_json_rpc_value(
            router.clone(),
            &auth_token,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "arroba.apply_patch",
                    "arguments": {
                        "patch_text": "*** Begin Patch\n*** Update File: notes.txt\n@@\n alpha\n-gamma\n+delta\n*** End Patch"
                    }
                }
            }),
        )
        .await
        .expect("patch request should succeed");
        assert_eq!(patch_response.status(), StatusCode::OK);
        let patch_body = patch_response
            .into_body()
            .collect()
            .await
            .expect("patch body should collect")
            .to_bytes();
        let patch_value: Value = serde_json::from_slice(&patch_body).expect("patch body json");
        assert_eq!(patch_value["result"]["structuredContent"]["applied"], true);
        assert!(patch_value["result"]["structuredContent"]["change"]["diff"]
            .as_str()
            .expect("patch diff should be present")
            .contains("+delta"));
        assert_eq!(
            std::fs::read_to_string(root.join("notes.txt")).expect("file should be readable"),
            "alpha\ndelta\n"
        );

        let write_response = handle_json_rpc_value(
            router.clone(),
            &auth_token,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "arroba.write_artifact",
                    "arguments": {
                        "path": "created.txt",
                        "content_text": "created through arroba\n"
                    }
                }
            }),
        )
        .await
        .expect("write request should succeed");
        assert_eq!(write_response.status(), StatusCode::OK);
        let write_body = write_response
            .into_body()
            .collect()
            .await
            .expect("write body should collect")
            .to_bytes();
        let write_value: Value = serde_json::from_slice(&write_body).expect("write body json");
        assert_eq!(write_value["result"]["structuredContent"]["applied"], true);
        assert_eq!(
            write_value["result"]["structuredContent"]["change"]["kind"],
            "add"
        );
        assert!(write_value["result"]["structuredContent"]["change"]["diff"]
            .as_str()
            .expect("write diff should be present")
            .contains("+created through arroba"));
        assert_eq!(
            std::fs::read_to_string(root.join("created.txt")).expect("file should be readable"),
            "created through arroba\n"
        );

        let move_delete_response = handle_json_rpc_value(
            router.clone(),
            &auth_token,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": {
                    "name": "arroba.apply_patch",
                    "arguments": {
                        "patch_text": "*** Begin Patch\n*** Update File: notes.txt\n*** Move to: archive/notes.txt\n@@\n-alpha\n+omega\n delta\n*** Delete File: created.txt\n*** End Patch"
                    }
                }
            }),
        )
        .await
        .expect("move/delete patch request should succeed");
        assert_eq!(move_delete_response.status(), StatusCode::OK);
        let move_delete_body = move_delete_response
            .into_body()
            .collect()
            .await
            .expect("move/delete body should collect")
            .to_bytes();
        let move_delete_value: Value =
            serde_json::from_slice(&move_delete_body).expect("move/delete body json");
        assert_eq!(
            move_delete_value["result"]["structuredContent"]["applied"],
            true
        );
        assert_eq!(
            std::fs::read_to_string(root.join("archive/notes.txt"))
                .expect("moved file should be readable"),
            "omega\ndelta\n"
        );
        assert!(!root.join("notes.txt").exists());
        assert!(!root.join("created.txt").exists());

        let rejected_patch_response = handle_json_rpc_value(
            router.clone(),
            &auth_token,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 6,
                "method": "tools/call",
                "params": {
                    "name": "arroba.apply_patch",
                    "arguments": {
                        "patch_text": "*** Begin Patch\n*** Add File: should-not-exist.txt\n+nope\n*** Update File: archive/notes.txt\n@@\n-missing\n+bad\n*** End Patch"
                    }
                }
            }),
        )
        .await
        .expect("rejected patch request should return a tool result");
        assert_eq!(rejected_patch_response.status(), StatusCode::OK);
        let rejected_patch_body = rejected_patch_response
            .into_body()
            .collect()
            .await
            .expect("rejected patch body should collect")
            .to_bytes();
        let rejected_patch_value: Value =
            serde_json::from_slice(&rejected_patch_body).expect("rejected patch body json");
        assert_eq!(
            rejected_patch_value["result"]["structuredContent"]["applied"],
            false
        );
        assert!(!root.join("should-not-exist.txt").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("archive/notes.txt"))
                .expect("moved file should remain unchanged"),
            "omega\ndelta\n"
        );

        let direct_move_response = handle_json_rpc_value(
            router.clone(),
            &auth_token,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "tools/call",
                "params": {
                    "name": "arroba.move_artifact",
                    "arguments": {
                        "from_path": "archive/notes.txt",
                        "to_path": "final.txt",
                        "old_text": "omega",
                        "new_text": "final"
                    }
                }
            }),
        )
        .await
        .expect("direct move request should succeed");
        assert_eq!(direct_move_response.status(), StatusCode::OK);
        let direct_move_body = direct_move_response
            .into_body()
            .collect()
            .await
            .expect("direct move body should collect")
            .to_bytes();
        let direct_move_value: Value =
            serde_json::from_slice(&direct_move_body).expect("direct move body json");
        assert_eq!(
            direct_move_value["result"]["structuredContent"]["applied"],
            true
        );
        assert_eq!(
            std::fs::read_to_string(root.join("final.txt")).expect("direct moved file should read"),
            "final\ndelta\n"
        );
        assert!(!root.join("archive/notes.txt").exists());

        let direct_delete_response = handle_json_rpc_value(
            router.clone(),
            &auth_token,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 8,
                "method": "tools/call",
                "params": {
                    "name": "arroba.delete_artifact",
                    "arguments": {
                        "path": "final.txt"
                    }
                }
            }),
        )
        .await
        .expect("direct delete request should succeed");
        assert_eq!(direct_delete_response.status(), StatusCode::OK);
        let direct_delete_body = direct_delete_response
            .into_body()
            .collect()
            .await
            .expect("direct delete body should collect")
            .to_bytes();
        let direct_delete_value: Value =
            serde_json::from_slice(&direct_delete_body).expect("direct delete body json");
        assert_eq!(
            direct_delete_value["result"]["structuredContent"]["applied"],
            true
        );
        assert_eq!(
            direct_delete_value["result"]["structuredContent"]["change"]["kind"],
            "delete"
        );
        assert!(!root.join("final.txt").exists());
    }

    #[tokio::test]
    async fn mcp_http_tools_call_lists_and_requests_capabilities() {
        let root = std::env::temp_dir().join(format!(
            "arroba-capability-mcp-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before unix epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join(".arroba").join("skills").join("browser-qa"))
            .expect("skill root should be created");
        std::fs::write(
            root.join(".arroba")
                .join("skills")
                .join("browser-qa")
                .join("SKILL.md"),
            "---\nname: browser-qa\ndescription: Browser QA\n---\nUse the browser.\n",
        )
        .expect("skill should be written");
        let mcp_registry =
            crate::mcp::ArrobaMcpRegistry::new(vec![root.join(".arroba").join("mcps")]);
        mcp_registry
            .install(&crate::mcp::ArrobaMcpServerConfig::stdio(
                "browser",
                "npx",
                vec!["@playwright/mcp".to_string()],
            ))
            .expect("mcp should install");

        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let workspace = root.to_string_lossy().to_string();
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(&workspace, &workspace))
            .expect("session should exist");
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("agent-a")
                    .with_model("test-model")
                    .with_worktree(&workspace),
            )
            .expect("agent should spawn");
        let agent_id = agent.id().to_string();
        let agent_ref = agent.agent_ref().to_string();
        let workflow_id = app
            .sessions_mut()
            .create_workflow(session.id(), Some("wf".to_string()))
            .expect("workflow should exist")
            .id()
            .to_string();
        let node_id = app
            .sessions_mut()
            .add_workflow_node(session.id(), &workflow_id, &agent_id)
            .expect("node should be added")
            .id()
            .to_string();
        app.sessions_mut()
            .create_workflow_endpoint(
                session.id(),
                &workflow_id,
                &node_id,
                Some("entry".to_string()),
            )
            .expect("endpoint should exist");
        app.invoke_workflow_endpoint_and_schedule(
            session.id(),
            &workflow_id,
            "entry",
            Some("start".to_string()),
        )
        .expect("workflow should invoke");
        let auth_token = app
            .providers()
            .get_run_for_agent(session.id(), &agent_id)
            .expect("provider run should exist")
            .runtime_mcp_auth_token()
            .expect("mcp auth token should exist")
            .to_string();

        let app = Arc::new(Mutex::new(app));
        let router = Arc::new(CommandRouter::with_interactive_capacity(app.clone(), 8));
        let list_response = handle_json_rpc_value(
            router.clone(),
            &auth_token,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "list_capabilities",
                    "arguments": {"kind": "all"}
                }
            }),
        )
        .await
        .expect("list request should succeed");
        let list_body = list_response
            .into_body()
            .collect()
            .await
            .expect("list body should collect")
            .to_bytes();
        let list_value: Value = serde_json::from_slice(&list_body).expect("list body json");
        assert_eq!(
            list_value["result"]["structuredContent"]["agent_ref"],
            agent_ref
        );
        assert_eq!(
            list_value["result"]["structuredContent"]["capabilities"]["mcps"][0]["name"],
            "browser"
        );
        assert_eq!(
            list_value["result"]["structuredContent"]["capabilities"]["skills"][0]["name"],
            "browser-qa"
        );

        let request_response = handle_json_rpc_value(
            router,
            &auth_token,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "request_capability",
                    "arguments": {"kind": "skill", "name": "browser-qa"}
                }
            }),
        )
        .await
        .expect("request capability should succeed");
        let request_body = request_response
            .into_body()
            .collect()
            .await
            .expect("request body should collect")
            .to_bytes();
        let request_value: Value =
            serde_json::from_slice(&request_body).expect("request body json");
        assert_eq!(
            request_value["result"]["structuredContent"]["granted"],
            true
        );
        assert_eq!(
            request_value["result"]["structuredContent"]["effective"],
            "now"
        );
        assert_eq!(
            request_value["result"]["structuredContent"]["requires_provider_restart"],
            false
        );
        assert!(
            request_value["result"]["structuredContent"]["skill"]["body"]
                .as_str()
                .expect("skill body should be returned")
                .contains("Use the browser.")
        );
        let agent = app
            .lock()
            .await
            .agents()
            .get_agent(&agent_id)
            .expect("agent should exist");
        assert_eq!(agent.skill_grants(), &["browser-qa".to_string()]);

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn mcp_tools_call_rejects_invalid_auth_token() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = Arc::new(CommandRouter::with_interactive_capacity(app, 8));
        let response = handle_json_rpc_value(
            router,
            "invalid-token",
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "ack_workflow_turn",
                    "arguments": {
                        "delivery_token": "workflow-ack:missing"
                    }
                }
            }),
        )
        .await
        .expect("request should return a json-rpc response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let value: Value = serde_json::from_slice(&body).expect("body should be json");
        assert_eq!(value["error"]["code"], -32000);
    }
}
