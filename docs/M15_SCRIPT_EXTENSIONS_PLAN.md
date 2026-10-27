# M15 Script Extensions Plan

M15 generalizes Chariox-managed provider add-ons under the name `extensions` and adds script extensions. Extension kinds for this phase are `mcp`, `skill`, and `script`; `api` is reserved for the next phase.

The goal is to let users expose local Python and TypeScript functions to agents as Chariox-managed tools without requiring provider-specific setup, a JSON manifest, or a Chariox package inside the user's environment.

## Design Decisions

- Extension grants are agent-scoped. Agents receive only extensions explicitly granted to them.
- MCP and skill management remains available through `/mcp` and `/skill`/`/skills`, but those commands are aliases over the generic extension model.
- Script extensions are regular script files. There is no user-authored JSON manifest in V1.
- Script environments are external only. Chariox records and validates existing Python or Node environments; it does not install Python, Node, npm packages, or virtualenv dependencies in V1.
- Runtime script environment is selected by the agent's script extension grant.
- Script execution is owned by a Chariox runner/shim. The implemented V1 runner is per-call and captures stdout/stderr separately from the returned payload; the intended next refinement is a warm turn-scoped runner to avoid repeated imports for heavy SDKs.
- Remote agents can use home-owned script extensions through the home-proxy extension path: the worker receives only the projected tool manifest, forwards invocations, and the home kernel validates the current grant/lease/provider-run binding before executing the script in the home script environment. Explicit worker-local script extensions still require the matching script and environment on the worker and must fail fast when missing.
- Workflows keep the current node-agent relationship. Workflow nodes use the extensions granted to their bound agents. Workflow export portability is deferred.

## Script Authoring Contract

One script file defines one script extension. The model-facing callable is always `run`; helpers may exist but are not exposed.

Python:

```python
def run(query: str, limit: int = 10) -> list[dict[str, object]]:
    """Search the internal vector index and return matching document snippets."""
    matches = search_index(query, limit)
    return matches

def test_run():
    result = run("toy query", limit=1)
    assert isinstance(result, list)
```

TypeScript:

```ts
/**
 * Search the internal vector index and return matching document snippets.
 */
export async function run(query: string, limit: number = 10): Promise<Record<string, unknown>> {
  const matches = await searchIndex(query, limit)
  return { matches, count: matches.length }
}

export async function test_run(): Promise<void> {
  const result = await run("toy query", 1)
  if (!result || typeof result !== "object") throw new Error("expected object")
}
```

Validation requirements:

- `run` exists.
- `test_run` exists and passes.
- `run` has a Python docstring or TypeScript JSDoc.
- Every `run` parameter has a supported type annotation.
- The return annotation is JSON-serializable.
- The returned value from `run` is JSON-serializable.

Supported V1 input types:

- Python: `str`, `int`, `float`, `bool`, `list[T]`, `dict`, and `Literal[...]`.
- TypeScript: `string`, `number`, `boolean`, arrays, simple record/object types, and literal unions.

Unsupported or ambiguous types fail validation with an actionable error.

## Output Contract

User scripts return data from `run`. They must not write model-facing results to stdout.

The Chariox runner owns stdout and serializes the returned value into the runtime tool result. User stdout/stderr or `console.log`/`console.error` output is captured as logs, not as the successful payload.

Successful tool result:

```json
{
  "ok": true,
  "payload": []
}
```

Error result:

```json
{
  "ok": false,
  "error": {
    "kind": "script_exception",
    "message": "run() raised ValueError: missing index"
  },
  "logs": "captured stdout/stderr"
}
```

Chariox enforces a per-call timeout and structured exception reporting in V1. Maximum returned payload size and captured log size caps are follow-up hardening work.

## CLI Surface

Environment commands:

```text
/env register <name> --python <path-to-python>
/env register <name> --node <path-to-node> [--package-root <path>]
/env list
/env show <name>
/env remove <name>
```

Script commands:

```text
/script validate <path> --env <env>
/script register <path> --env <env> [--name <name>]
/script list
/script show <name>
/script remove <name>
```

Extension commands:

```text
/extension grant mcp <agent-ref> <name>
/extension grant skill <agent-ref> <name>
/extension grant script <agent-ref> <name> --env <env>
/extension revoke <kind> <agent-ref> <name>
/extension grants <kind> <agent-ref>
```

`/mcp`, `/skill`, and `/skills` remain thin aliases over `/extension` operations.

## Protocol Changes

This is a breaking protocol change.

- Increment `LOCAL_DAEMON_PROTOCOL_VERSION`.
- Replace separate `mcp_grants` and `skill_grants` serialized agent fields with:

```text
extension_grants: Vec<ExtensionGrant>
```

- Replace grant kind types with:

```text
ExtensionKind = mcp | skill | script
ExtensionGrant {
  kind: ExtensionKind
  name: string
  environment: string?  // required for script grants
}
```

- Replace runtime tool terminology:
  - `list_capabilities` -> `list_extensions`
  - `request_capability` -> `request_extension`

## Live Drill Matrix

- Python env lifecycle: register/list/show/remove; invalid executable rejected.
- Node env lifecycle: register/list/show/remove; invalid executable rejected; TypeScript runner probe passes when `tsx` is available in the registered environment.
- TypeScript env lifecycle: `.ts` script validation passes when `tsx` is available in the registered Node environment/package root.
- Python script lifecycle: validate/register/list/show/remove realistic vector-search fixture; missing `test_run`, missing docstring, and missing type hints fail.
- TypeScript script lifecycle: same lifecycle for `.ts`.
- Local provider Python script use: grant script to Agent A only; Agent A sees/calls it; Agent B cannot.
- Local provider TypeScript script use: same as Python.
- Warm runner follow-up: script records import count; two calls in one turn import once and run twice; runner exits after turn.
- Error handling: script exception produces structured tool error and captured logs without corrupting the provider turn.
- Schema enforcement follow-up: invalid arguments are rejected before script execution.
- Output enforcement follow-up: JSON-serializable values succeed; non-serializable return and oversized output fail clearly.
- Workflow script use: node's bound agent has script grant; workflow run calls script and completes.
- Home-proxy remote negative: grant is missing, revoked, stale, or bound to the wrong leased agent/provider run; home rejects the invocation and `/extension audit` reports the denial.
- Home-proxy remote positive: worker lacks the script/env locally, receives only the projected manifest, remote agent invokes the script, and output proves execution happened on home with credentials/env kept home-local.
- Worker-local remote negative: worker lacks script/env and fails fast with remediation before provider execution.
- Worker-local remote positive: same script and env are installed on worker; remote agent calls script successfully and output confirms worker-side execution.
- Alias coverage: `/mcp` and `/skill` aliases still work; `/extension grants` shows equivalent grants.
