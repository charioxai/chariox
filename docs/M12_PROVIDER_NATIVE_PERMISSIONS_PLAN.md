# M12 Provider-Native Permissions And Popups

## Goal

Add Arroba-managed user-facing permissions without introducing Arroba-owned bash execution in v1.

Arroba will:

- normalize provider-native permission behavior,
- expose one consistent user model,
- surface native permission/question requests through Arroba clients,
- support defaults at the session level and overrides at the agent level.

MCP gating is out of scope for this milestone because MCP access is already controlled by Arroba grants.

## V1 user model

Two orthogonal knobs:

- `mode`: `build | plan`
- `permissions`: `required | yolo`

Scope:

- session default via `/session mode` and `/session permissions`
- agent override via `/agent mode` and `/agent permissions`

Precedence:

- agent override wins
- otherwise session default applies
- otherwise provider defaults are used

## Provider mapping

### Codex

- `build + required` -> approval `on-request`, sandbox `workspace-write`
- `build + yolo` -> approval `never`, sandbox `danger-full-access`
- `plan + required` -> approval `on-request`, sandbox `read-only`
- `plan + yolo` -> approval `never`, sandbox `read-only`

Managed-I/O launch policy remains authoritative when enabled and can still force stricter behavior.

### OpenCode

- `mode` maps to `default_agent = "build" | "plan"`
- `permissions=required` maps native tool permissions to `ask`
- `permissions=yolo` maps native tool permissions to `allow`

V1 applies the permission mapping to:

- `edit`
- `bash`
- `task`

## Kernel/runtime work

1. Extend provider launch/runtime state with:
   - `execution_mode`
   - `permission_level`
2. Store optional agent overrides on `AgentInstance`.
3. Allow `SpawnAgentRequest` to carry optional overrides.
4. Add `UpdateAgentConfig` local request/response.
5. Resolve effective values from:
   - agent override
   - session config (`agents.mode`, `agents.permissions`)
   - default fallback
6. Use effective values at provider launch and relaunch.

## Client surface

### CLI

Commands:

- `/session mode [build|plan]`
- `/session permissions [required|yolo]`
- `/agent mode [agent-ref] [build|plan|inherit]`
- `/agent permissions [agent-ref] [required|yolo|inherit]`

Behavior:

- no value shows the current effective value
- `inherit` clears the agent override and falls back to the session value

### Shell

Commands mirror the CLI forms without the slash:

- `session mode ...`
- `session permissions ...`
- `agent mode ...`
- `agent permissions ...`

`context` should show the current effective mode and permissions for the selected agent.

## Footer/UI

Agent footers should include:

- identity
- provider
- model
- effort
- effective mode
- effective permissions

This is display-only state derived from effective resolution, not from raw override fields alone.

## Popups

This slice does not yet add the popup interaction system itself.

It prepares the permission/mode model that the popup work will sit on top of. The popup layer will normalize provider-native permission/question requests into Arroba client surfaces in a follow-up slice.

## Landed local slice

Implemented:

- kernel/runtime support for session defaults plus agent overrides
- provider-native mapping for Codex and OpenCode
- shell commands for session/agent mode and permissions
- CLI slash commands for session/agent mode and permissions
- agent footer display of effective mode and permissions

Validated with:

- `cargo test --manifest-path apps/kernel/Cargo.toml --no-run`
- `pnpm --filter @arroba/kernel-client run test`
- `pnpm --filter @arroba/cli run lint`

## Deferred

- popup-based permission/question response UX
- remote permission UX drills
- provider-native live permission drills
- any stricter Arroba-owned execution gate
