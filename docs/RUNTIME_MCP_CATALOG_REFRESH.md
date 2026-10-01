# MP-08/MP-10 provider runtime tool catalog refresh

The kernel's provider-facing MCP endpoint negotiates supported 2025 MCP
versions and advertises `tools.listChanged=true`. An authenticated GET opens a
Streamable HTTP SSE stream. Script/connector registration/removal, agent
extension grant/revoke and leased manifest updates invalidate discovery. Each
stream compares only its authenticated effective catalog (names, descriptions
and schemas); unrelated or identical grants produce no notification. Changes
emit `notifications/tools/list_changed`, without tool or agent data. A
catalog-changing tool call also emits the notification before its result on its
POST SSE stream. Slow connections have bounded queues; disconnect and token
retirement close their stream ownership.

OpenCode 1.18.23 refreshes definitions using its official notification handler:
[versioned MCP handler](https://github.com/anomalyco/opencode/blob/v1.18.23/packages/opencode/src/mcp/index.ts).
Codex 0.159.3 only logs the notification:
[versioned client handler](https://github.com/openai/codex/blob/rust-v0.159.3/codex-rs/rmcp-client/src/logging_client_handler.rs).
OpenCode definition refresh is asynchronous; the reserved-builder API drill
completed its active turn without invoking the newly granted tool. Handler
existence alone therefore does not establish immediate active-turn visibility.
Codex, OpenCode, Claude and unverified providers conservatively report
`effective=after_provider_reload`, `requires_provider_restart=true` for runtime
script/connector grants. The existing kernel provider reload and durable resume
path refreshes the conversation after the active turn, with the existing
kernel-owned automation attachment through normal prompt admission, so
disconnecting the original client cannot cancel continuation ownership. An idle catalog change reloads without replaying an
already completed prompt. No provider SDK, visible PTY injection, or alternative
prompt authority is added. Notifications remain available to official provider
clients; immediate grant claims require verified active-turn behavior. Skill body grants retain their immediate behavior; external
MCP configuration grants retain their existing reload behavior.

MP-08/MP-10 wire compatibility: local daemon version 371 binds this MCP behavior
change. LocalDaemon request/response and relay shapes are unchanged, so their
snapshot hashes remain unchanged. This increment is compatible with release
C's envlayer3 additive protocol371 changes; the aggregate must retain that
lane's updated hashes and relay version65. Existing B client minimum370 remains
appropriate because these notifications are consumed by provider MCP clients.
MCP negotiation rejects an unsupported modern revision by selecting supported
2025-03-26, whose GET/POST SSE behavior this endpoint implements.

Focused HTTP tests exercise authenticated GET, registration before grant,
grant/revoke/remove/reregister, schema changes and unchanged catalogs. The
reserved-builder extfix evidence records exact source/artifact identities,
fail-first results, live Codex/OpenCode checks, resource samples and cleanup.
These checks do not close MP-08/MP-10, independent review, signed release C,
hosted client or fresh Path-1 acceptance.
