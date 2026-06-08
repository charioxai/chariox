# M16 Runtime MCP Extension Registration Plan

M16 lets agents use the Arroba runtime MCP to register Arroba-managed extensions on behalf of the user. Registered extensions must be indistinguishable from user-created extensions: they live in Arroba's global extension registries, appear in every Arroba terminal, can be managed by normal Arroba extension commands and panels, and can be granted to agents through the existing grant/revoke model.

This plan covers registration only. It does not auto-grant newly registered extensions. Agents that have enough runtime permission can register an extension, then explicitly call `arroba.request_extension` to use it.

## Decisions

- Extension registries are global-only. Arroba should not segment extension definitions by project/workspace; grant/revoke remains the access-control boundary.
- The kernel remains the extension authority. Cloud and relay do not store, proxy, or interpret extension definitions.
- The runtime MCP gets registration tools for MCPs, skills, environments, scripts, connectors, and connector adapters.
- Scripts, skills, connectors, and connector adapters are registered by path. Agents can create files with normal provider-native shell/file tools, then register the resulting path with Arroba.
- MCPs and environments are registered from structured config because their current registry APIs already use structured definitions.
- Registration does not auto-grant.
- Existing `AgentPermissionLevel` is the permission taxonomy:
  - `yolo`: registration proceeds without extra approval.
  - `required`: registration creates a kernel `RuntimeInteraction` approval before persistence.
- Keep `yolo` as the default permission level.
- MCP grants are the only path that needs provider warm relaunch/continuation, because MCPs are rendered into provider-native MCP config at provider launch. Skills need no hot reload. Scripts and connectors are Arroba runtime MCP tools and are not provider-native registrations.
- The existing MCP warm relaunch/continuation path should cover Codex, OpenCode, and Claude. M16 must validate Claude end to end and fix only if the drill proves the current path is incomplete.

## Architecture Fit

Runtime MCP calls enter the kernel through the existing authenticated provider-run MCP surface. The new registration tools should dispatch under the same runtime tool bridge as `arroba.list_extensions` and `arroba.request_extension`.

For local agents:

```text
provider tool call -> runtime MCP -> home kernel -> global extension registry
```

For remote agents:

```text
worker provider tool call -> worker runtime MCP -> relay -> home kernel -> global extension registry
```

The worker must not become an extension registry authority. A remote registration request is a forwarded control-plane request to home, where home applies the session/agent permission policy and writes the global registry.

Cloud remains a terminal/control-plane client:

```text
browser -> hosted relay -> kernel -> existing extension list requests
```

Cloud only needs to render the installed extensions it already receives from the kernel.

## Registry Scope

Change normal extension discovery/registration to use user-global roots only:

```text
~/.arroba/mcps
~/.arroba/skills
~/.arroba/envs
~/.arroba/scripts
~/.arroba/connectors/definitions
~/.arroba/connectors/adapters
```

Existing `workspace_id` fields should be retained for request compatibility and relative path resolution only. For example, `register_script_path` may resolve `scripts/foo.py` against the current workspace/worktree, but the installed script definition is written to `~/.arroba/scripts`.

Project-local roots such as `./.arroba/mcps` and `./.arroba/skills` should no longer participate in normal extension discovery/registration after this milestone. If backwards compatibility with existing project-local definitions is needed, handle it with an explicit import/migration path rather than keeping project-local roots in the main registry search path.

## Runtime MCP Tool Surface

Add canonical tools plus provider-safe aliases where the existing runtime MCP alias pattern requires them.

### `arroba.register_mcp`

Registers or updates a global Arroba MCP definition.

Input:

```json
{
  "config": {
    "name": "echo",
    "transport": {
      "type": "stdio",
      "command": "node",
      "args": ["/path/to/server.mjs"]
    },
    "enabled": true,
    "required": false
  }
}
```

Behavior:

- Validate with the existing `ArrobaMcpServerConfig` validation.
- Write to the global MCP registry.
- Return the installed definition and path.
- Do not grant it to the current agent.

### `arroba.register_skill_path`

Registers or updates a global skill from a directory containing `SKILL.md`.

Input:

```json
{
  "path": "skills/review"
}
```

Behavior:

- Resolve relative paths against the current workspace/worktree context.
- Validate with the existing skill metadata parser.
- Install/update into `~/.arroba/skills`.
- Return metadata and path.

### `arroba.register_environment`

Registers or updates a global script execution environment.

Input:

```json
{
  "config": {
    "name": "py",
    "runtime": {
      "type": "python",
      "python": "/usr/bin/python3"
    }
  }
}
```

Behavior:

- Validate executable/runtime with the existing environment registry logic.
- Write to `~/.arroba/envs`.
- Return environment config and path.

### `arroba.register_script_path`

Registers a global script extension from a Python or TypeScript file.

Input:

```json
{
  "path": "tools/lookup.py",
  "environment": "py",
  "name": "lookup"
}
```

Behavior:

- Resolve relative paths against the current workspace/worktree context.
- Resolve the environment from the global environment registry.
- Validate with the existing script inspector and `test_run`.
- Install into `~/.arroba/scripts`.
- Return script metadata and path.

### `arroba.register_connector_path`

Registers or updates a global connector from connector YAML.

Input:

```json
{
  "path": "connectors/status-api/connector.yaml"
}
```

Behavior:

- Resolve relative paths against the current workspace/worktree context.
- Validate with the existing connector registry and adapter validation.
- Write to `~/.arroba/connectors/definitions`.
- Return connector definition and path.

### `arroba.register_connector_adapter_path`

Registers or updates a global connector adapter from an adapter YAML/package.

Input:

```json
{
  "path": "connectors/http-adapter/adapter.yaml"
}
```

Behavior:

- Resolve relative paths against the current workspace/worktree context.
- Validate with the existing adapter registry logic.
- Copy/register into `~/.arroba/connectors/adapters`.
- Return adapter definition and path.

## Permission Gate

Add a shared runtime-tool permission gate for persistent extension registry mutations.

Inputs:

- session id
- agent id
- operation name
- extension kind/name when known
- resolved source path or target path when known
- effective `AgentPermissionLevel`

Behavior:

- If permission is `yolo`, proceed.
- If permission is `required`, create a `RuntimeInteraction` with `Allow` and `Deny`.
- Denial returns a structured runtime tool failure and does not mutate the registry.
- Approval proceeds with the registry mutation.
- The interaction must be projected to all Arroba terminals attached to the session, including web terminal and remote TUI, through existing runtime interaction projection.

Suggested interaction text:

```text
Allow agent `<agent-ref>` to register global Arroba <kind> `<name>`?
```

When the operation is path-based, include the resolved source path in the message.

## Grant And Use Semantics

Registration and grants remain separate.

After registration:

- The extension appears in `arroba.list_extensions`.
- The extension appears in normal CLI/web extension lists.
- It is not exposed to an agent until granted.

For use:

- MCP: agent calls `arroba.request_extension`; existing grant flow records the grant, schedules MCP continuation, reloads the provider after the current turn, and resumes the prompt with hidden continuation context. Validate for Codex, OpenCode, and Claude.
- Skill: agent calls `arroba.request_extension`; skill body/package is returned/materialized and can be followed immediately.
- Script: agent calls `arroba.request_extension` with `environment`; script tool becomes an Arroba runtime MCP tool for the agent.
- Connector: agent calls `arroba.request_extension` with optional credential and safety; connector operations become Arroba runtime MCP tools for the agent.

Do not add provider reloads for skills, scripts, or connectors unless a focused end-to-end drill proves a provider-specific blocker. The intended architecture is that only third-party MCP grants require provider-native relaunch.

## Implementation Steps

1. Add global-only registry helpers for runtime extension discovery and registration.
2. Update existing extension listing paths to use global roots only.
3. Keep request `workspace_id` and session workspace context only for resolving relative registration source paths.
4. Add runtime MCP tool specs and canonical alias mapping.
5. Add a new runtime dispatch module for extension registration tools.
6. Reuse existing registry install/upsert/validation APIs.
7. Add shared permission gating for persistent extension registration mutations.
8. Forward remote worker registration calls to home through the existing remote capability control-plane path or a minimal sibling request if the existing response shape is insufficient.
9. Ensure registration results republish or refresh any needed session/agent projections so terminals can update without restart.
10. Update docs and tests.
11. Add live drills and artifact capture.

## Tests

Unit and integration coverage:

- Runtime MCP tool specs include all registration tools and aliases.
- Each registration tool validates and writes to the global registry.
- Relative source paths resolve from the session workspace/worktree but write globally.
- Project-local roots are not searched by normal extension listing after the global-only change.
- `yolo` permission registers without an interaction.
- `required` permission denial blocks registry mutation.
- `required` permission approval permits registry mutation.
- Remote worker registration forwards to home and mutates only home global registries.
- Collaborator remote agents cannot mutate home-owned registries unless acting through an authorized home-owned path.
- Registration does not auto-grant.
- After registration plus grant:
  - MCP uses existing warm continuation path.
  - Skill can be consumed immediately.
  - Script can be called through runtime MCP.
  - Connector can be called through runtime MCP.

Protocol considerations:

- If new behavior only adds runtime MCP tool names and payloads, update runtime MCP/tool snapshot tests and docs.
- If `LocalDaemonRequest`, `LocalDaemonResponse`, relay terminal events, browser/kernel terminal transport, or serialized relay peer shapes change, increment `LOCAL_DAEMON_PROTOCOL_VERSION` and update protocol snapshot/hash tests.

## Live End-To-End Drills

All drills should write logs and screenshots under `./.artifacts`.

### `live-runtime-register-skill-drill.mjs`

Flow:

1. Create a temporary skill directory with `SKILL.md`.
2. Start a local provider agent.
3. Agent calls `arroba.register_skill_path`.
4. Agent calls `arroba.list_extensions` and sees the skill as ungranted.
5. Agent calls `arroba.request_extension`.
6. Agent uses the returned skill body in the same turn.
7. Open/list extensions from the terminal and confirm visibility.

Screenshot:

```text
./.artifacts/runtime-register-skill-terminal.png
```

### `live-runtime-register-script-drill.mjs`

Flow:

1. Create a Python or TypeScript script file with valid `run` and `test_run`.
2. Register an environment through `arroba.register_environment`.
3. Register the script through `arroba.register_script_path`.
4. Agent calls `arroba.list_extensions` and sees the script as ungranted.
5. Agent calls `arroba.request_extension` with the environment.
6. Agent calls the script runtime tool in the same provider session.

Screenshot:

```text
./.artifacts/runtime-register-script-terminal.png
```

### `live-runtime-register-connector-drill.mjs`

Flow:

1. Create a connector adapter package/YAML.
2. Register it through `arroba.register_connector_adapter_path`.
3. Create connector YAML.
4. Register it through `arroba.register_connector_path`.
5. Agent calls `arroba.list_extensions` and sees the connector as ungranted.
6. Agent calls `arroba.request_extension`.
7. Agent calls a connector operation in the same provider session.

Screenshot:

```text
./.artifacts/runtime-register-connector-terminal.png
```

### `live-runtime-register-mcp-drill.mjs`

Run for Codex, OpenCode, and Claude.

Flow:

1. Create a deterministic echo MCP server fixture.
2. Register it through `arroba.register_mcp`.
3. Agent calls `arroba.list_extensions` and sees the MCP as ungranted.
4. Agent calls `arroba.request_extension`.
5. Verify existing warm provider relaunch/continuation occurs.
6. After continuation, agent calls the MCP successfully.

Screenshots:

```text
./.artifacts/runtime-register-mcp-codex-continuation.png
./.artifacts/runtime-register-mcp-opencode-continuation.png
./.artifacts/runtime-register-mcp-claude-continuation.png
```

### `live-runtime-register-permission-drill.mjs`

Flow:

1. Run a `yolo` agent registration and verify no approval appears.
2. Run a `required` agent registration and deny approval; verify registry is unchanged.
3. Run a `required` agent registration and approve; verify registry is changed.

Screenshot:

```text
./.artifacts/runtime-register-required-approval.png
```

### `live-runtime-global-visibility-drill.mjs`

Flow:

1. Register an extension from workspace A.
2. Open workspace B/session B.
3. Confirm the extension appears in `arroba.list_extensions` and normal extension listing.
4. Confirm it is ungranted in workspace B.
5. Grant it in workspace B and use it.

Screenshot:

```text
./.artifacts/runtime-global-visibility-workspace-b.png
```

### Cloud Sidebar Drill

Add or update an `arroba-cloud` drill.

Flow:

1. Launch the web terminal against a kernel with the M16 runtime registration tools.
2. Register an extension through an agent runtime MCP call.
3. Open or refresh the Extensions sidebar.
4. Confirm the installed global extension appears without Cloud-specific registry mutation logic.

Screenshot:

```text
./.artifacts/cloud-extensions-sidebar-after-runtime-install.png
```

## Acceptance Criteria

- Agents can register MCPs, skills, environments, scripts, connectors, and connector adapters through runtime MCP.
- Registered extensions are global and visible from any Arroba terminal.
- Registration is permission-gated by the existing agent permission level.
- Default `yolo` behavior remains unchanged.
- Registration does not auto-grant.
- MCP registration plus request uses warm relaunch/continuation for Codex, OpenCode, and Claude.
- Skills, scripts, and connectors do not rely on provider-native registration.
- Cloud only renders kernel-projected installed extensions.
- All live drills pass and save screenshots under `./.artifacts`.
