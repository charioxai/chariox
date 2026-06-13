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

- `build + approval-required` -> approval `untrusted`, sandbox `workspace-write`
- `build + yolo` -> approval `never`, sandbox `danger-full-access`
- `plan + approval-required` -> approval `untrusted`, sandbox `read-only`
- `plan + yolo` -> approval `never`, sandbox `read-only`

Workspace live sync launch policy remains authoritative when enabled. Managed mode can force stricter behavior when selective write fencing is unavailable. Tracked mode is observational, so Codex build runs use `danger-full-access` with the same native approval policy; this keeps repositories outside the selected synced roots editable instead of letting Codex's `workspace-write` sandbox block them before Arroba can observe the selected roots at turn end.

Codex CLI documentation also refers to the strict approval policy as `unless-trusted`; the current
app-server schema used by Arroba's supported Codex runtime names the same mode `untrusted`.

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

M12 includes the blocking popup interaction system itself.

Delivered in this milestone:

- `request_popup` as a runtime MCP tool
- blocking same-turn interaction resolution
- provider-native permission requests normalized into the same interaction model
- focused-pane CLI interaction strips for choices and approvals
- provider answer routing back to Codex/OpenCode native approval paths

## Landed

Implemented:

- kernel/runtime support for session defaults plus agent overrides
- provider-native mapping for Codex and OpenCode
- shell commands for session/agent mode and permissions
- CLI slash commands for session/agent mode and permissions
- agent footer display of effective mode and permissions
- blocking popup interactions via `request_popup`
- always-injected shared runtime instructions advertising Arroba runtime tools
- Codex native approval interception and response routing
- OpenCode native permission interception and response routing
- pane-local CLI interaction rendering for user feedback and provider-native permissions
- remote forwarding for both provider-native interactions and non-permission `request_popup` interactions
- live popup, native-permission, remote popup, and remote permission drill scripts

Validated with:

- `cargo test --manifest-path apps/kernel/Cargo.toml --no-run`
- `pnpm --filter @arroba/kernel-client run test`
- `pnpm --filter @arroba/cli run lint`
- controlled-exec spike fake/live drills for blocking popup semantics
- live native permission drills for Codex and OpenCode
- live popup drills, including Codex popup execution on the real-home provider auth path
- remote native permission drills for Codex and OpenCode
- remote workspace live sync permission drills for Codex and OpenCode
- remote non-permission popup drills for Codex and OpenCode

## Closed Scope

M12 is closed with:

- session-level and agent-level `mode` / `permissions`
- agent override precedence over session defaults
- normalized provider-native permission UX for Codex and OpenCode
- blocking Arroba popup UX for user feedback
- same-turn continuation after popup or native permission response
- CLI interaction-strip rendering and response submission
- remote leased-agent forwarding for provider-native permissions and non-permission popups

The following are intentionally outside M12 and belong to later milestones:

- shell popup queue UX
- any Arroba-owned execution gate beyond provider-native permissions
