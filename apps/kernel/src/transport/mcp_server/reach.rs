//! The `chariox-reach` MCP server at `/mcp/reach`: a second tool level next to
//! `chariox`, served with the same per-run bearer. Its tools exist for the kernel
//! owner's agents only; the room-level `chariox` list never changes.
use super::*;

pub(super) async fn handle(
    router: Arc<CommandRouter>,
    request: Request<Incoming>,
) -> Result<Response<HttpBody>, DaemonError> {
    if request.method() != Method::POST {
        return Ok(empty_response(StatusCode::METHOD_NOT_ALLOWED));
    }
    let Some(auth_token) = parse_bearer_token(request.headers()) else {
        return Ok(text_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized".to_string(),
        ));
    };
    let body = request
        .into_body()
        .collect()
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "reach_mcp_read_body",
            message: error.to_string(),
        })?
        .to_bytes();
    let payload =
        serde_json::from_slice::<Value>(&body).map_err(|error| DaemonError::LocalTransport {
            operation: "reach_mcp_parse_json",
            message: error.to_string(),
        })?;
    let id = payload.get("id").cloned();
    let Some(method) = payload.get("method").and_then(Value::as_str) else {
        return Ok(json_rpc_error_response(id, -32600, "invalid request"));
    };
    if id.is_none() {
        return Ok(empty_response(StatusCode::ACCEPTED));
    }
    let result = match method {
        "initialize" => {
            let protocol_version = payload
                .get("params")
                .and_then(|params| params.get("protocolVersion"))
                .and_then(Value::as_str)
                .filter(|version| matches!(*version, "2025-03-26" | "2025-06-18" | "2025-11-25"))
                .unwrap_or(DEFAULT_PROTOCOL_VERSION);
            serde_json::json!({
                "protocolVersion": protocol_version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "chariox-reach", "version": env!("CARGO_PKG_VERSION")},
            })
        }
        "tools/list" => serde_json::json!({
            "tools": router
                .reach_tool_specs_for_auth_token(&auth_token)
                .into_iter()
                .map(|tool| serde_json::json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                }))
                .collect::<Vec<_>>()
        }),
        "tools/call" => {
            let params = payload.get("params").cloned().unwrap_or(Value::Null);
            let Some(tool_name) = params.get("name").and_then(Value::as_str) else {
                return Ok(json_rpc_error_response(id, -32602, "missing tool name"));
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            match router
                .dispatch_reach_tool_call(&auth_token, tool_name, arguments)
                .await
            {
                Ok(result) => {
                    let (content, structured_content) =
                        super::tool_content::runtime_tool_content(result.payload);
                    serde_json::json!({
                        "content": content,
                        "structuredContent": structured_content,
                        "isError": !result.ok,
                    })
                }
                Err(error) => {
                    return Ok(json_rpc_error_response(
                        id,
                        -32000,
                        &tool_errors::message(&error),
                    ))
                }
            }
        }
        "ping" => serde_json::json!({}),
        _ => return Ok(json_rpc_error_response(id, -32601, "method not found")),
    };
    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({"jsonrpc": JSON_RPC_VERSION, "id": id, "result": result}),
    ))
}
