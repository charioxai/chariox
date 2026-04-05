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
use tokio::sync::Mutex;

use crate::app::DaemonApp;
use crate::error::DaemonError;

const DEFAULT_PROTOCOL_VERSION: &str = "2025-03-26";
const JSON_RPC_VERSION: &str = "2.0";

type HttpBody = Full<Bytes>;

pub async fn run_mcp_http_server(app: Arc<Mutex<DaemonApp>>) -> Result<(), DaemonError> {
    let (bind_host, bind_port) = {
        let app = app.lock().await;
        (
            app.config().runtime_mcp_host.clone(),
            app.config().runtime_mcp_port,
        )
    };
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
        let app = Arc::clone(&app);
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service = service_fn(move |request| {
                let app = Arc::clone(&app);
                async move { handle_http_request(app, request).await }
            });
            let _ = http1::Builder::new().serve_connection(io, service).await;
        });
    }
}

async fn handle_http_request(
    app: Arc<Mutex<DaemonApp>>,
    request: Request<Incoming>,
) -> Result<Response<HttpBody>, Infallible> {
    let response = match handle_http_request_inner(app, request).await {
        Ok(response) => response,
        Err(error) => text_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("runtime MCP server error: {error}"),
        ),
    };
    Ok(response)
}

async fn handle_http_request_inner(
    app: Arc<Mutex<DaemonApp>>,
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

    if request.uri().path() != "/mcp" {
        return Ok(text_response(
            StatusCode::NOT_FOUND,
            "not found".to_string(),
        ));
    }

    match *request.method() {
        Method::GET => Ok(empty_response(StatusCode::METHOD_NOT_ALLOWED)),
        Method::POST => handle_json_rpc_request(app, request).await,
        Method::DELETE => Ok(empty_response(StatusCode::METHOD_NOT_ALLOWED)),
        _ => Ok(empty_response(StatusCode::METHOD_NOT_ALLOWED)),
    }
}

async fn handle_json_rpc_request(
    app: Arc<Mutex<DaemonApp>>,
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
    handle_json_rpc_value(app, &auth_token, payload).await
}

async fn handle_json_rpc_value(
    app: Arc<Mutex<DaemonApp>>,
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
                    "tools": crate::transport::runtime_tools::workflow_runtime_tool_specs()
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
        "tools/call" => {
            let params = payload.get("params").cloned().unwrap_or(Value::Null);
            let Some(tool_name) = params.get("name").and_then(Value::as_str) else {
                return Ok(json_rpc_error_response(id, -32602, "missing tool name"));
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let result = {
                let mut app = app.lock().await;
                crate::transport::runtime_tools::dispatch_authenticated_runtime_tool_call(
                    &mut app, auth_token, tool_name, arguments,
                )
            };
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

    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::local::{
        AddWorkflowNodeRequest, CreateWorkflowEndpointRequest, CreateWorkflowRequest,
        InvokeWorkflowEndpointRequest, LocalDaemonRequest, SpawnAgentRequest,
    };
    use crate::session::CreateSessionRequest;
    use crate::{DaemonApp, DaemonConfig};

    use super::handle_json_rpc_value;

    #[tokio::test]
    async fn mcp_http_tools_call_acknowledges_active_workflow_turn() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session should exist")
        {
            crate::local::LocalDaemonResponse::SessionCreated { session, agent } => {
                (session, agent)
            }
            other => panic!("unexpected response: {other:?}"),
        };
        app.attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");
        let agent_id = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("agent-a".to_string()),
                provider: "dev-stub".to_string(),
                model: Some("test-model".to_string()),
                effort: None,
                worktree_id: Some("worktree-1".to_string()),
            }))
            .expect("agent should spawn")
        {
            crate::local::LocalDaemonResponse::AgentSpawned { agent } => agent.id().to_string(),
            other => panic!("unexpected response: {other:?}"),
        };
        let workflow_id = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some("wf".to_string()),
            }))
            .expect("workflow should exist")
        {
            crate::local::LocalDaemonResponse::WorkflowCreated { workflow, .. } => {
                workflow.id().to_string()
            }
            other => panic!("unexpected response: {other:?}"),
        };
        let node_id = match app
            .handle_local_request(LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow_id.clone(),
                    agent_id: agent_id.clone(),
                },
            ))
            .expect("node should be added")
        {
            crate::local::LocalDaemonResponse::WorkflowNodeAdded { node, .. } => {
                node.id().to_string()
            }
            other => panic!("unexpected response: {other:?}"),
        };
        match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflowEndpoint(
                CreateWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow_id.clone(),
                    entry_node_id: node_id,
                    alias: Some("entry".to_string()),
                },
            ))
            .expect("endpoint should exist")
        {
            crate::local::LocalDaemonResponse::WorkflowEndpointCreated { .. } => {}
            other => panic!("unexpected response: {other:?}"),
        }
        let workflow_run = match app
            .handle_local_request(LocalDaemonRequest::InvokeWorkflowEndpoint(
                InvokeWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow_id,
                    endpoint_ref: "entry".to_string(),
                    prompt: Some("start".to_string()),
                },
            ))
            .expect("workflow should invoke")
        {
            crate::local::LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => {
                workflow_run
            }
            other => panic!("unexpected response: {other:?}"),
        };
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
        let response = handle_json_rpc_value(
            app.clone(),
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
}
