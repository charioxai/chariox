# Config Command Audit

Date: 2026-04-27

This audits the user-config command surface exposed by:

- Terminal CLI: `/config show`, `/config path`, `/config keys`, `/config schema`, `/config set`, `/config unset`, `/config workspace-live-sync`
- Web/shell CLI: `config show`, `config path`, `config keys`, `config schema`, `config set`, `config unset`, `config workspace-live-sync`
- Kernel IPC: `GetUserConfig`, `GetUserConfigSchema`, `SetUserConfigValue`, `UnsetUserConfigValue`

The main product issue is that `set` and `unset` are generic dotted-key escape hatches. They expose parser branches even when the parsed value is not actually consumed by runtime code.

## Command Surface

| Command | Used | Current behavior | Recommendation | Provider hot reload |
|---|---:|---|---|---|
| `/config show` / `config show` | Yes | Fetches and prints persisted user config. | Keep. It is the only complete user-visible config inspection path. | No. |
| `/config path` / `config path` | Yes | Prints the user config file path. | Keep. Useful for TOML-only sections like credentials. | No. |
| `/config keys` / `config keys` | Yes | Fetches kernel-owned config schema and prints settable keys with type/status/effect metadata. | Keep. This is the discoverable parameter list for generic `set/unset`. | No. |
| `/config schema` / `config schema` | Yes | Fetches kernel-owned config schema and prints full JSON metadata. | Keep. Useful for web/automation and debugging key behavior. | No. |
| `/config workspace-live-sync` / `config workspace-live-sync` | Yes | Writes global `providers.workspace_live_sync` launch policy through `SetUserConfigValue`; supported values are `off`, `managed`, and `tracked`. Active sessions can also change mode with `workspace sync off|managed|tracked`. | Keep as the friendly wrapper over global workspace live sync launch policy. | Yes, implemented through central provider reload policy. |
| `/config set <path> <value>` / `config set <path> <value>` | Partially | Writes any supported parser key, including stale/dead keys; the kernel now returns mutation-effect metadata for provider reload, restart-required, and currently-unwired keys. | Keep only if paired with discoverable docs and key allowlist/status; otherwise it still creates false expectations for keys with no runtime behavior. | Depends on key. |
| `/config unset <path>` / `config unset <path>` | Partially | Unsets supported parser keys; some backend enum keys reject unset. The same mutation-effect path is used as `set`. | Same as `set`. | Depends on key. |

## Key Audit

Status meanings:

- `keep`: config is actively consumed and should remain configurable.
- `fix`: intent looks valid, but current implementation is incomplete or misleading.
- `remove`: key looks dead, duplicated by another config system, or too aspirational for the current product.
- `restart`: config is real, but only effective on daemon restart.

| Key | Command exposed | Status | What it does today | Recommendation | Provider hot reload |
|---|---:|---|---|---|---|
| `providers.workspace_live_sync` | Yes | keep | Controls the default Workspace Live Sync mode for new provider launches: `off`, `managed`, or `tracked`. Active running providers are reloaded or deferred through the central reload policy. | Keep as the single global launch policy; session mode stays owned by `workspace sync off|managed|tracked`. | Yes, implemented. |
| `providers.default` | Yes | fix/remove | Parsed and persisted, but launch defaults appear to come from CLI/session paths, not this key. | Either wire as the kernel-owned default provider or remove from config commands. | Only if wired and only for agents/runs that inherited the default. |
| `providers.model` | Yes | fix/remove | Parsed and persisted, but not clearly used as launch default. | Same as `providers.default`. | Only if wired and inherited. |
| `providers.account_profile` | Yes | fix/remove | Parsed and persisted, but launch requests pass explicit account profile. | Same as `providers.default`. | Only if wired and inherited. |
| `providers.effort` | Yes | fix/remove | Parsed and persisted, but not clearly used as variant/effort default. | Same as `providers.default`. | Only if wired and inherited. |
| `version` | Set only | remove from command | Internal schema version can be set manually through generic command. Unset is blocked. | Do not expose through config command; keep TOML/load migration-owned. | No. |
| `ui.theme` | Yes | remove/fix | Parsed in kernel config, but terminal UI uses CLI preferences/theme registry state. | Prefer CLI preferences as owner; remove kernel key unless browser terminal needs kernel-owned UI defaults. | No. |
| `ui.multi_agent_response_layout` | Yes | remove/fix | Parsed in kernel config, but response layout is driven by CLI preferences/session config. | Prefer CLI preferences/session config; remove kernel key or wire intentionally. | No. |
| `ui.max_agents_per_screen` | Yes | remove/fix | Parsed in kernel config, but CLI has its own max-agents state/preference path. | Prefer CLI preferences; remove kernel key or wire intentionally. | No. |
| `ui.worktree_aliases.*` | Yes, via worktree alias command | fix/remove | CLI writes aliases, but no read path was found in current search. | If aliases are desired, wire waiting room/worktree labels to read this; otherwise remove. | No. |
| `relay.url` | Yes | remove/fix | Parsed into `user_config.relay`, but runtime relay connection uses top-level persisted/env `relay_url`. | Remove from user config or wire it into `DaemonConfig::load_from_env` and relay reconnect policy. | No provider reload. Relay reconnect needed if wired. |
| `relay.accept_remote_leases` | Yes | remove/fix | Parsed into `user_config.relay`, but remote lease acceptance uses top-level `accept_remote_leases` from env/config struct. | Remove or wire into effective daemon config. | No provider reload. Relay/lease policy reload needed if wired. |
| `kernel.websocket_host` | Yes | remove/fix | Parsed into user config, but websocket bind URL uses top-level `kernel_websocket_host` from env/default. | Remove from user config or wire at daemon boot only. | No. Daemon restart if wired. |
| `kernel.websocket_port` | Yes | remove/fix | Same as websocket host. | Same as websocket host. | No. Daemon restart if wired. |
| `kernel.runtime_mcp_host` | Yes | remove/fix | Parsed into user config, but runtime MCP URL uses top-level `runtime_mcp_host` from env/default. | Remove from user config or wire at daemon boot with clear restart semantics. | If live-wired, provider reload would be required, but daemon restart is simpler. |
| `kernel.runtime_mcp_port` | Yes | remove/fix | Same as runtime MCP host. | Same as runtime MCP host. | If live-wired, provider reload would be required, but daemon restart is simpler. |
| `history.operational.backend` | Set only, unset blocked | keep as placeholder | Only legal value is `sqlite`; store opens on daemon boot. | Keep if backend expansion is planned; otherwise hide from command help. | No. Restart required for backend changes. |
| `history.operational.path` | Yes | keep/restart | Used to open the operational history SQLite DB on daemon boot. | Keep, but surface restart-required. | No. Restart required. |
| `history.operational.retention_days` | Yes | fix/remove | Parsed and validated; no clear pruning job using it was found. | Wire retention pruning or remove. | No. |
| `history.operational.max_size_mb` | Yes | fix/remove | Parsed and validated; no clear size pruning job using it was found. | Wire size pruning or remove. | No. |
| `history.operational.keep_pinned_sessions` | Yes | fix/remove | Parsed; no clear pruning logic using it was found. | Wire into pruning or remove with retention keys. | No. |
| `history.operational.archive_inactive_after_days` | Yes | fix/remove | Parsed; no clear automatic inactive archival job using it was found. | Wire archival scheduler or remove. | No. |
| `history.operational.archive_deleted_agents` | Yes | fix/remove | Parsed; no clear behavior using it was found. | Wire or remove. | No. |
| `history.archive.mode` | Yes | keep | Enables external archive queue/search behavior when set to `external`. | Keep. | No. |
| `history.archive.url` | Yes | keep | Required endpoint for external history archive. | Keep. | No. |
| `history.archive.token_env` | Yes | keep | Optional env var for archive bearer token. | Keep. | No. |
| `history.archive.require_durable_acceptance` | Yes | keep | Used by archive client to require every event be accepted. | Keep. | No. |
| `history.archive.archive_deleted_agents` | Yes | fix/remove | Parsed; no clear use found. | Wire into deletion/archive flow or remove. | No. |
| `history.archive.archive_before_delete` | Yes | fix/remove | Parsed; no clear use found. | Wire into deletion/archive flow or remove. | No. |
| `history.archive.delete_operational_after_verified_archive` | Yes | fix/remove | Parsed; no clear use found. | Wire into archive flush/prune flow or remove. | No. |
| `artifacts.operational.backend` | Set only, unset blocked | keep as placeholder | Only legal value is `filesystem`; artifact store opens when used. | Keep if backend expansion is planned; otherwise hide from command help. | No. |
| `artifacts.operational.root` | Yes | keep | Filesystem root for operational artifacts. Used when storing transferred artifacts. | Keep. | No provider reload. Existing store calls use current config at operation time. |
| `artifacts.operational.index_path` | Yes | keep | SQLite index path for operational artifacts. | Keep. | No provider reload. |
| `artifacts.operational.retention_days` | Yes | fix/remove | Parsed and validated; no clear cleanup job using it was found. | Wire cleanup or remove. | No. |
| `artifacts.archive.mode` | Yes | keep | Used by archive flush tooling for artifact archive export. | Keep if archive flush remains supported. | No. |
| `artifacts.archive.url` | Yes | keep | External artifact archive endpoint. | Keep. | No. |
| `artifacts.archive.token_env` | Yes | keep | Optional env var for artifact archive bearer token. | Keep. | No. |
| `artifacts.archive.require_durable_acceptance` | Yes | keep | Used by artifact archive client. | Keep. | No. |
| `state.backend` | Set only, unset blocked | keep as placeholder | Only legal value is `sqlite`; durable state opens on daemon boot. | Keep if backend expansion is planned; otherwise hide from command help. | No. Restart required. |
| `state.path` | Yes | keep/restart | Durable kernel state DB path. Used at daemon boot. | Keep, but surface restart-required. | No. Restart required. |
| `state.snapshot_interval_events` | Yes | keep | Used when deciding when to save durable snapshots. | Keep. | No. |
| `credential_vault.service` | Yes | keep | Selects OS-keychain service namespace for vault-backed credentials. Runtime credential tools read the current config snapshot. | Keep. Warn users it changes which vault namespace is used. | No provider reload. |
| `credentials` | TOML only | keep/fix command gap | Defines credential handles. Used by runtime credential tools and provider env scrubbing. Not editable through generic `config set`. | Keep TOML support. Add structured credential-handle commands if we want this managed from CLI. | Yes if live-edited, because provider env scrub list can change. |
| MCP servers/grants | No; separate `/mcp` | keep separate | MCP configuration is managed outside user config. Provider reload is already handled through MCP grant flow. | Keep separate from `/config`. | Yes, implemented through policy for grants. |

## Recommended Cleanup Plan

1. Keep `show`, `path`, `workspace-live-sync`, and generic `set/unset`. Generic mutations now return/display effect metadata for `provider_reload`, `restart_required`, and `no_runtime_effect`; keep expanding the table as more keys are intentionally wired.
2. Remove command support for `version`.
3. Decide ownership for provider defaults. If kernel owns them, wire `providers.default/model/account_profile/effort` into session/provider launch defaulting and track whether values were inherited. If CLI owns them, remove the kernel config keys.
4. Delete or wire kernel `ui.*`. Right now CLI preferences appear to be the real owner.
5. Delete or wire user-config `relay.*` and `kernel.*`; the runtime currently uses top-level env/persisted daemon config for these.
6. Either implement pruning/archive jobs for retention and deletion-policy keys, or remove those keys until the behavior exists.
7. Keep provider hot reload narrow for now: `providers.workspace_live_sync`, MCP grants, and future live credential-handle edits. Do not hot reload providers for UI, history, artifact, relay, state, or daemon socket/websocket settings.
