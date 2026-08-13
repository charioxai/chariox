use super::*;

pub fn meta_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    vec![
        RuntimeToolSpec {
            name: META_SESSION_OVERVIEW_TOOL.to_string(),
            description: "Return a compact overview of the current session for this agent in Meta mode: owned agents, agent status, workflow state, pending interactions, and event counts.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "include_workflows": {"type": "boolean"},
                    "include_events": {"type": "boolean"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_SEARCH_COMMANDS_TOOL.to_string(),
            description: "Search Chariox commands available to this agent in Meta mode by natural-language goal, name, usage, intent, tag, scope, mutation behavior, or Meta mode policy.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "tag": {"type": "string"},
                    "scope": {"type": "string", "enum": ["session", "global", "external"]},
                    "mutates": {"type": "boolean"},
                    "policy": {"type": "string", "enum": ["allow", "approval", "deny"]},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_LIST_COMMANDS_TOOL.to_string(),
            description: "List Chariox commands available to this agent in Meta mode, with optional filtering by tag, scope, mutation behavior, or policy.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "tag": {"type": "string"},
                    "scope": {"type": "string", "enum": ["session", "global", "external"]},
                    "mutates": {"type": "boolean"},
                    "policy": {"type": "string", "enum": ["allow", "approval", "deny"]},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_COMMAND_DOCS_TOOL.to_string(),
            description: "Return exact usage, examples, tags, scope, mutation behavior, and policy for one Chariox command.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["command"],
                "properties": {
                    "command": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_SEARCH_GUIDES_TOOL.to_string(),
            description: "Search concise Chariox operational guides for workflows, agent apps, events, supervision, and common failures.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "tag": {"type": "string"},
                    "command": {"type": "string"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_LIST_GUIDES_TOOL.to_string(),
            description: "List concise Chariox operational guides, optionally filtered by tag or command reference.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "tag": {"type": "string"},
                    "command": {"type": "string"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_READ_GUIDE_TOOL.to_string(),
            description: "Read one Chariox operational guide by id or exact title, including its Markdown body.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["guide"],
                "properties": {
                    "guide": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_RUN_COMMAND_TOOL.to_string(),
            description: "Run one allowed Chariox command inside this session as this agent in Meta mode. Session creation, cross-session targeting, and self-approval are denied.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["command"],
                "properties": {
                    "command": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_LIST_EVENTS_TOOL.to_string(),
            description: "List this agent's Meta mode event inbox records. Event prompts are visible runtime prompts; this tool is for replay and detail lookup.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100},
                    "status": {"type": "string"},
                    "kind": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_READ_EVENT_TOOL.to_string(),
            description: "Read full detail for one Meta mode event by event id.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["event_id"],
                "properties": {"event_id": {"type": "string"}},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_ACK_EVENT_TOOL.to_string(),
            description: "Acknowledge one or more Meta mode events for bookkeeping and replay control.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "event_id": {"type": "string"},
                    "event_ids": {"type": "array", "items": {"type": "string"}},
                    "up_to_sequence": {"type": "integer", "minimum": 0}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_TURN_OVERVIEW_TOOL.to_string(),
            description: "Return an ordered overview of a turn trace: assistant messages, reasoning entries, tool calls, tool results, status, and errors.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_ref": {"type": "string"},
                    "turn_ref": {"type": "string"},
                    "turns_back": {"type": "integer", "minimum": 0},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_TURN_BLOB_TOOL.to_string(),
            description: "Return exact content for a selected turn blob when policy allows it.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["blob_id"],
                "properties": {"blob_id": {"type": "string"}},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_SUBSCRIBE_TRACE_TOOL.to_string(),
            description: "Attach this agent in Meta mode to the live terminal stream for one owned regular agent. Subscribe before prompting the worker so provider output is routed to the supervision stream.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["agent_ref"],
                "properties": {
                    "agent_ref": {"type": "string"},
                    "mode": {"type": "string", "enum": ["compact", "verbose"]}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_POLL_TRACE_TOOL.to_string(),
            description: "Drain currently buffered live trace records from a Meta mode supervision stream without waiting. Compact mode returns summaries and short excerpts; verbose mode returns capped raw text. Use wait_trace for normal worker supervision.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subscription_id": {"type": "string"},
                    "agent_ref": {"type": "string"},
                    "mode": {"type": "string", "enum": ["compact", "verbose"]},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WAIT_TRACE_TOOL.to_string(),
            description: "Wait briefly for live worker trace records, then drain them. Prefer this after prompting a worker: it blocks until activity, worker output, completion, error, or timeout instead of returning an empty snapshot immediately.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subscription_id": {"type": "string"},
                    "agent_ref": {"type": "string"},
                    "mode": {"type": "string", "enum": ["compact", "verbose"]},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100},
                    "wait_ms": {"type": "integer", "minimum": 1, "maximum": 60000},
                    "until": {"type": "string", "enum": ["any", "activity", "worker_output", "completion", "error"]}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_UNSUBSCRIBE_TRACE_TOOL.to_string(),
            description: "Detach a Meta mode live trace subscription and discard any pending compact stream records for it.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["subscription_id"],
                "properties": {"subscription_id": {"type": "string"}},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_SUBSCRIBE_EVENTS_TOOL.to_string(),
            description: format!(
                "Subscribe this agent in Meta mode to an optional session event. Valid event kinds: {}.",
                META_EVENT_KINDS.join(", ")
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["kind"],
                "properties": {
                    "kind": {"type": "string", "enum": META_EVENT_KINDS},
                    "filter": {"type": "object"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_UNSUBSCRIBE_EVENTS_TOOL.to_string(),
            description: "Remove an optional Meta mode event subscription. Required agent turn and interaction subscriptions cannot be removed.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["subscription_id"],
                "properties": {"subscription_id": {"type": "string"}},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_LIST_SUBSCRIPTIONS_TOOL.to_string(),
            description: "List required and optional event subscriptions for this agent in Meta mode.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_READ_TASK_TOOL.to_string(),
            description: "Read this agent's kernel-managed Meta mode task document and status. Returns status none when no task exists.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_UPDATE_TASK_TOOL.to_string(),
            description: "Update this agent's kernel-managed Meta mode task markdown. Creates the task if it does not exist.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["markdown"],
                "properties": {
                    "markdown": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_READ_PLAN_TOOL.to_string(),
            description: "Read this agent's kernel-managed Meta mode plan markdown and task status.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_UPDATE_PLAN_TOOL.to_string(),
            description: "Update this agent's kernel-managed Meta mode plan markdown. Creates an empty active task if needed.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["markdown"],
                "properties": {
                    "markdown": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_COMPLETE_TASK_TOOL.to_string(),
            description: "Mark this agent's active Meta mode task completed with an optional summary.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "summary": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_MARK_BLOCKED_TOOL.to_string(),
            description: "Mark this agent's Meta mode task blocked with the concrete reason progress cannot continue.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["reason"],
                "properties": {
                    "reason": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_CREATE_TOOL.to_string(),
            description: "Create a saved workflow-code artifact in this session from JS/TS source after kernel compilation and validation. node_path is optional; the kernel discovers Node.js when omitted.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["name", "source"],
                "properties": {
                    "name": {"type": "string"},
                    "source": {"type": "string"},
                    "language": {"type": "string", "enum": ["javascript", "java_script", "typescript", "type_script"]},
                    "node_path": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_READ_TOOL.to_string(),
            description: "Read one saved workflow-code artifact, including source, compiled workflow definition, and validation metadata.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["name"],
                "properties": {"name": {"type": "string"}},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_LIST_TOOL.to_string(),
            description: "List saved workflow-code artifacts visible to this session.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_UPDATE_TOOL.to_string(),
            description: "Update a saved workflow-code artifact after recompiling and validating the supplied JS/TS source.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["name", "source"],
                "properties": {
                    "name": {"type": "string"},
                    "source": {"type": "string"},
                    "language": {"type": "string", "enum": ["javascript", "java_script", "typescript", "type_script"]},
                    "node_path": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_DELETE_TOOL.to_string(),
            description: "Delete one saved workflow-code artifact visible to this session.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["name"],
                "properties": {"name": {"type": "string"}},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_VALIDATE_TOOL.to_string(),
            description: "Validate workflow-code without mutating session workflow state. Pass either saved artifact name or inline source; node_path is optional for inline source.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "source": {"type": "string"},
                    "language": {"type": "string", "enum": ["javascript", "java_script", "typescript", "type_script"]},
                    "node_path": {"type": "string"},
                    "provider_rebindings": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["node", "provider"],
                            "properties": {
                                "node": {"type": "string"},
                                "provider": {"type": "string"},
                                "model": {"type": "string"},
                                "effort": {"type": "string"},
                                "account_profile": {"type": "string"}
                            },
                            "additionalProperties": false
                        }
                    },
                    "agent_rebindings": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["node", "agent_ref"],
                            "properties": {
                                "node": {"type": "string"},
                                "agent_ref": {"type": "string"}
                            },
                            "additionalProperties": false
                        }
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_APPLY_TOOL.to_string(),
            description: "Apply saved or inline workflow-code into the current session. Applying creates a new workflow with fresh kernel ids and generated agents as needed.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "source": {"type": "string"},
                    "language": {"type": "string", "enum": ["javascript", "java_script", "typescript", "type_script"]},
                    "node_path": {"type": "string"},
                    "provider_rebindings": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["node", "provider"],
                            "properties": {
                                "node": {"type": "string"},
                                "provider": {"type": "string"},
                                "model": {"type": "string"},
                                "effort": {"type": "string"},
                                "account_profile": {"type": "string"}
                            },
                            "additionalProperties": false
                        }
                    },
                    "agent_rebindings": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["node", "agent_ref"],
                            "properties": {
                                "node": {"type": "string"},
                                "agent_ref": {"type": "string"}
                            },
                            "additionalProperties": false
                        }
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_RUN_TOOL.to_string(),
            description: "Apply saved or inline workflow-code into the current session and invoke one endpoint. endpoint may be a script endpoint handle or a kernel endpoint ref; when omitted, the script must define exactly one endpoint.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "source": {"type": "string"},
                    "language": {"type": "string", "enum": ["javascript", "java_script", "typescript", "type_script"]},
                    "prompt": {"type": "string", "description": "Invocation prompt. When omitted or blank, the workflow-code script-level prompt is used; if the script has no prompt, Chariox uses a generic run instruction."},
                    "endpoint": {"type": "string"},
                    "queue": {"type": "string"},
                    "node_path": {"type": "string"},
                    "provider_rebindings": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["node", "provider"],
                            "properties": {
                                "node": {"type": "string"},
                                "provider": {"type": "string"},
                                "model": {"type": "string"},
                                "effort": {"type": "string"},
                                "account_profile": {"type": "string"}
                            },
                            "additionalProperties": false
                        }
                    },
                    "agent_rebindings": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["node", "agent_ref"],
                            "properties": {
                                "node": {"type": "string"},
                                "agent_ref": {"type": "string"}
                            },
                            "additionalProperties": false
                        }
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_EXPORT_TOOL.to_string(),
            description: "Export a saved workflow-code artifact as a portable package without local filesystem paths.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_IMPORT_TOOL.to_string(),
            description: "Import a portable workflow-code package after checking package integrity and validating the embedded workflow definition on this kernel. name overrides the package name; overwrite replaces an existing saved artifact with the target name.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["package"],
                "properties": {
                    "package": {"type": "object"},
                    "name": {"type": "string"},
                    "overwrite": {"type": "boolean"},
                    "node_path": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_PACKAGE_EXPORT_TOOL.to_string(),
            description: "Export a saved workflow-code artifact as a portable workflow-code package. This is the explicit package-named form; chariox.meta.workflow_code.export is kept as a compatibility alias.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_PACKAGE_IMPORT_TOOL.to_string(),
            description: "Import a portable workflow-code package after integrity checking and validation. This is the explicit package-named form; chariox.meta.workflow_code.import is kept as a compatibility alias.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["package"],
                "properties": {
                    "package": {"type": "object"},
                    "name": {"type": "string"},
                    "overwrite": {"type": "boolean"},
                    "node_path": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_SOURCE_EXPORT_TOOL.to_string(),
            description: "Export a saved workflow-code artifact as source. format inline returns the saved JS/TS source; format directory returns workflow.js plus schemas/*.json and manifest.json contents.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": {"type": "string"},
                    "format": {"type": "string", "enum": ["inline", "directory"]}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_SOURCE_EXPORT_DIRECTORY_TOOL.to_string(),
            description: "Export a saved workflow-code artifact as a source directory package with workflow.js, external schemas/*.json files, and manifest.json hashes. This is the explicit directory-named form of source_export with format directory.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_CODE_CANVAS_CONTRACT_TOOL.to_string(),
            description: "Return the authoritative workflow-code canvas dimensions and spacing contract for nodes, endpoints, generated exit markers, recommended grid placement, and validation scope.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_REGISTRY_LIST_TOOL.to_string(),
            description: "List reusable workflow registry entries visible to this session. Precedence is workspace, then user, then builtin.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_REGISTRY_GET_TOOL.to_string(),
            description: "Read metadata for one reusable workflow registry entry by name.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["name"],
                "properties": {"name": {"type": "string"}},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_REGISTRY_ADD_TOOL.to_string(),
            description: "Register a reusable workflow from workflow-code source. Use kind single_file with source, or kind source_directory with files containing workflow.js, schemas/*.json, and optional manifest.json.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["name", "source"],
                "properties": {
                    "name": {"type": "string"},
                    "scope": {"type": "string", "enum": ["workspace", "user"]},
                    "node_path": {"type": "string"},
                    "source": {
                        "oneOf": [
                            {
                                "type": "object",
                                "required": ["kind", "source"],
                                "properties": {
                                    "kind": {"const": "single_file"},
                                    "source": {"type": "string"},
                                    "source_path": {"type": "string"}
                                },
                                "additionalProperties": false
                            },
                            {
                                "type": "object",
                                "required": ["kind", "files"],
                                "properties": {
                                    "kind": {"const": "source_directory"},
                                    "files": {
                                        "type": "array",
                                        "items": {
                                            "type": "object",
                                            "required": ["path", "contents", "sha256"],
                                            "properties": {
                                                "path": {"type": "string"},
                                                "contents": {"type": "string"},
                                                "sha256": {"type": "string"}
                                            },
                                            "additionalProperties": false
                                        }
                                    }
                                },
                                "additionalProperties": false
                            }
                        ]
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_REGISTRY_ADD_FROM_WORKFLOW_TOOL.to_string(),
            description: "Register a reusable workflow from an existing live workflow. Portable generated-agent source is used by default; existing agent refs are preserved only with agent_mode existing_agents.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["name", "workflow_ref"],
                "properties": {
                    "name": {"type": "string"},
                    "workflow_ref": {"type": "string"},
                    "scope": {"type": "string", "enum": ["workspace", "user"]},
                    "agent_mode": {"type": "string", "enum": ["portable_generated", "existing_agents"]}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_REGISTRY_DELETE_TOOL.to_string(),
            description: "Delete a user or workspace workflow registry entry. Builtin registry entries cannot be deleted.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": {"type": "string"},
                    "scope": {"type": "string", "enum": ["workspace", "user"]}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_REGISTRY_LOAD_TOOL.to_string(),
            description: "Load a registered workflow into the current session. This validates and applies the workflow-code, creating fresh workflow/node/edge ids and generated agents as needed.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": {"type": "string"},
                    "parameters": {
                        "type": "object",
                        "description": "Workflow-code template input parameters, using keys from the registry entry's parameters_schema."
                    },
                    "provider_rebindings": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["node", "provider"],
                            "properties": {
                                "node": {"type": "string"},
                                "provider": {"type": "string"},
                                "model": {"type": "string"},
                                "effort": {"type": "string"},
                                "account_profile": {"type": "string"}
                            },
                            "additionalProperties": false
                        }
                    },
                    "agent_rebindings": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["node", "agent_ref"],
                            "properties": {
                                "node": {"type": "string"},
                                "agent_ref": {"type": "string"}
                            },
                            "additionalProperties": false
                        }
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_WORKFLOW_REGISTRY_RUN_TOOL.to_string(),
            description: "Load a registered workflow into the current session and invoke one endpoint. endpoint and queue are workflow-code handles from the registered source.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": {"type": "string"},
                    "parameters": {
                        "type": "object",
                        "description": "Workflow-code template input parameters, using keys from the registry entry's parameters_schema."
                    },
                    "prompt": {"type": "string"},
                    "endpoint": {"type": "string"},
                    "queue": {"type": "string"},
                    "provider_rebindings": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["node", "provider"],
                            "properties": {
                                "node": {"type": "string"},
                                "provider": {"type": "string"},
                                "model": {"type": "string"},
                                "effort": {"type": "string"},
                                "account_profile": {"type": "string"}
                            },
                            "additionalProperties": false
                        }
                    },
                    "agent_rebindings": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["node", "agent_ref"],
                            "properties": {
                                "node": {"type": "string"},
                                "agent_ref": {"type": "string"}
                            },
                            "additionalProperties": false
                        }
                    }
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: META_RESOLVE_RUNTIME_INTERACTION_TOOL.to_string(),
            description: "Resolve a kernel-owned runtime interaction for one of this user's regular agents. An agent in Meta mode can never resolve its own interactions.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["interaction_id"],
                "properties": {
                    "interaction_id": {"type": "string"},
                    "choice_id": {"type": "string"},
                    "input": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
    ]
}
