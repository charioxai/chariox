//! The options of an agent HTTP trigger: the Workflows panel's HTTP trigger fields.
//! The gateways route and parse a request with them; the kernel checks the input schema.
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A sync response waits this long when the request sets no timeout, as a sync publication does.
pub const DEFAULT_SYNC_TIMEOUT_MS: u64 = 300_000;
/// A sync response reads the reply this often when the request sets no poll, as a publication does.
pub const DEFAULT_POLL_MS: u64 = 500;
const MIN_POLL_MS: u64 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum AgentHttpMethod {
    Get,
    Post,
}

/// How a request becomes the trigger's input, as the publication parsers do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentHttpParser {
    /// "Path prompt": the `:prompt` segment of the route.
    PathTemplate {
        template: String,
    },
    Json,
    QueryParams,
    /// Headers, body and query together.
    Webhook,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentHttpResponseMode {
    Async,
    Sync,
}

/// Async answers once the firing is accepted; sync waits for the agent's reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum AgentHttpResponse {
    Async,
    Sync { sync_timeout_ms: u64, poll_ms: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentHttpOptions {
    pub route: String,
    pub methods: Vec<AgentHttpMethod>,
    pub parser: AgentHttpParser,
    #[serde(flatten)]
    pub response: AgentHttpResponse,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
}

/// The options as a client sends them; anything left out takes the publication default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentHttpOptionsRequest {
    pub route: Option<String>,
    pub methods: Vec<AgentHttpMethod>,
    pub parser: Option<AgentHttpParser>,
    pub mode: Option<AgentHttpResponseMode>,
    pub sync_timeout_ms: Option<u64>,
    pub poll_ms: Option<u64>,
    pub input_schema: Option<Value>,
}

impl AgentHttpOptions {
    pub fn from_request(request: AgentHttpOptionsRequest) -> Result<Self, String> {
        let route = request.route.as_deref().map_or("/", str::trim).to_string();
        check_path("route", &route)?;
        let mut methods = if request.methods.is_empty() {
            vec![AgentHttpMethod::Get, AgentHttpMethod::Post]
        } else {
            request.methods
        };
        methods.sort();
        methods.dedup();
        let parser = request.parser.unwrap_or(AgentHttpParser::Json);
        if let AgentHttpParser::PathTemplate { template } = &parser {
            check_path("path template", template)?;
            if !template.split('/').any(|segment| segment == ":prompt") {
                return Err("a path prompt template needs a `:prompt` segment".into());
            }
        }
        let response = match request.mode.unwrap_or(AgentHttpResponseMode::Async) {
            AgentHttpResponseMode::Async => {
                if request.sync_timeout_ms.is_some() || request.poll_ms.is_some() {
                    return Err("Timeout ms and Poll ms apply only to a sync response".into());
                }
                AgentHttpResponse::Async
            }
            AgentHttpResponseMode::Sync => {
                let sync_timeout_ms = request.sync_timeout_ms.unwrap_or(DEFAULT_SYNC_TIMEOUT_MS);
                if !(1..=DEFAULT_SYNC_TIMEOUT_MS).contains(&sync_timeout_ms) {
                    return Err(format!(
                        "Timeout ms must be a positive integer up to {DEFAULT_SYNC_TIMEOUT_MS}"
                    ));
                }
                let poll_ms = request
                    .poll_ms
                    .unwrap_or(DEFAULT_POLL_MS.min(sync_timeout_ms));
                if poll_ms < MIN_POLL_MS || poll_ms > sync_timeout_ms {
                    return Err(format!(
                        "Poll ms must be between {MIN_POLL_MS} and the timeout"
                    ));
                }
                AgentHttpResponse::Sync {
                    sync_timeout_ms,
                    poll_ms,
                }
            }
        };
        if let Some(schema) = &request.input_schema {
            compile_input_schema(schema)?;
        }
        Ok(Self {
            route,
            methods,
            parser,
            response,
            input_schema: request.input_schema,
        })
    }

    /// Refuses a request whose parsed input does not match the input schema.
    pub fn check_input(&self, input: &Value) -> Result<(), String> {
        let Some(schema) = &self.input_schema else {
            return Ok(());
        };
        let compiled = compile_input_schema(schema)?;
        if let Err(errors) = compiled.validate(input) {
            let first = errors
                .into_iter()
                .next()
                .map(|error| error.to_string())
                .unwrap_or_default();
            return Err(format!("input does not match the input schema: {first}"));
        }
        Ok(())
    }
}

fn compile_input_schema(schema: &Value) -> Result<jsonschema::JSONSchema, String> {
    if !schema.is_object() {
        return Err("Input schema must be a JSON Schema object".into());
    }
    jsonschema::JSONSchema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .compile(schema)
        .map_err(|error| format!("Input schema is not a valid JSON Schema: {error}"))
}

fn check_path(label: &str, path: &str) -> Result<(), String> {
    if !path.starts_with('/') {
        return Err(format!("agent HTTP trigger {label} must start with `/`"));
    }
    if path.len() > 256
        || path
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '?' | '#' | '*' | '\\'))
        || path.split('/').any(|segment| segment == "..")
        || path.contains("//")
    {
        return Err(format!(
            "agent HTTP trigger {label} `{path}` must be a plain path of at most 256 characters"
        ));
    }
    // The trigger page and its gateway own these addresses below the trigger's base.
    if let Some(reserved) = path.split('/').nth(1).filter(|first| {
        matches!(
            *first,
            "assets" | "events" | "health" | "history" | "resend" | "send" | "workspace-image"
        )
    }) {
        return Err(format!(
            "agent HTTP trigger {label} cannot start with `/{reserved}`: the trigger page uses it"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> AgentHttpOptionsRequest {
        AgentHttpOptionsRequest::default()
    }

    #[test]
    fn left_out_options_take_the_publication_defaults() {
        let options = AgentHttpOptions::from_request(request()).unwrap();
        assert_eq!(options.route, "/");
        assert_eq!(
            options.methods,
            vec![AgentHttpMethod::Get, AgentHttpMethod::Post]
        );
        assert_eq!(options.parser, AgentHttpParser::Json);
        assert_eq!(options.response, AgentHttpResponse::Async);
        let sync = AgentHttpOptions::from_request(AgentHttpOptionsRequest {
            mode: Some(AgentHttpResponseMode::Sync),
            ..request()
        })
        .unwrap();
        assert_eq!(
            sync.response,
            AgentHttpResponse::Sync {
                sync_timeout_ms: DEFAULT_SYNC_TIMEOUT_MS,
                poll_ms: DEFAULT_POLL_MS
            }
        );
    }

    #[test]
    fn invalid_options_are_refused_with_what_to_change() {
        for (bad, message) in [
            (
                AgentHttpOptionsRequest {
                    route: Some("hooks".into()),
                    ..request()
                },
                "must start with `/`",
            ),
            (
                AgentHttpOptionsRequest {
                    route: Some("/a/../b".into()),
                    ..request()
                },
                "plain path",
            ),
            (
                AgentHttpOptionsRequest {
                    route: Some("/history/today".into()),
                    ..request()
                },
                "the trigger page uses it",
            ),
            (
                AgentHttpOptionsRequest {
                    parser: Some(AgentHttpParser::PathTemplate {
                        template: "/run/:name".into(),
                    }),
                    ..request()
                },
                "`:prompt` segment",
            ),
            (
                AgentHttpOptionsRequest {
                    sync_timeout_ms: Some(1000),
                    ..request()
                },
                "only to a sync response",
            ),
            (
                AgentHttpOptionsRequest {
                    mode: Some(AgentHttpResponseMode::Sync),
                    sync_timeout_ms: Some(0),
                    ..request()
                },
                "Timeout ms must be a positive integer",
            ),
            (
                AgentHttpOptionsRequest {
                    mode: Some(AgentHttpResponseMode::Sync),
                    sync_timeout_ms: Some(1_000),
                    poll_ms: Some(5_000),
                    ..request()
                },
                "Poll ms must be between 50 and the timeout",
            ),
            (
                AgentHttpOptionsRequest {
                    mode: Some(AgentHttpResponseMode::Sync),
                    poll_ms: Some(10),
                    ..request()
                },
                "Poll ms must be between 50 and the timeout",
            ),
            (
                AgentHttpOptionsRequest {
                    input_schema: Some(serde_json::json!(["not", "an", "object"])),
                    ..request()
                },
                "must be a JSON Schema object",
            ),
            (
                AgentHttpOptionsRequest {
                    input_schema: Some(serde_json::json!({"type": "objekt"})),
                    ..request()
                },
                "not a valid JSON Schema",
            ),
        ] {
            let error = AgentHttpOptions::from_request(bad.clone()).expect_err("refused");
            assert!(error.contains(message), "{bad:?}: {error}");
        }
    }

    #[test]
    fn the_input_schema_refuses_a_non_conforming_input() {
        let options = AgentHttpOptions::from_request(AgentHttpOptionsRequest {
            input_schema: Some(serde_json::json!({
                "type": "object", "required": ["ticket"],
                "properties": {"ticket": {"type": "integer"}},
            })),
            ..request()
        })
        .unwrap();
        options
            .check_input(&serde_json::json!({"ticket": 42}))
            .unwrap();
        for input in [
            serde_json::json!({}),
            serde_json::json!({"ticket": "42"}),
            serde_json::json!("text"),
        ] {
            let error = options.check_input(&input).expect_err("refused");
            assert!(
                error.starts_with("input does not match the input schema"),
                "{error}"
            );
        }
    }
}
