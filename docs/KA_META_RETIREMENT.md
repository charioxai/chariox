# MP-08 / MP-11 — proposed Meta retirement (PR 9)

This is the coordinator's proposed default for the release after `/sudo`.
The owner must confirm the alias decision before this branch ships. It removes
`/meta` and its task controls from the shared terminal catalog and CLI. A typed
`/meta`, with or without a task, is refused by the kernel with `/sudo` guidance
before creating a turn or materializing attachments. It never forwards the task
or elevates it automatically. No replacement delegation-only command is added.

Meta tools are absent from runtime MCP discovery. Guessed legacy aliases are
refused at both the kernel/router admission and forwarded-worker entry point,
including persisted legacy Meta agents. Sudo keeps its existing
`chariox_kernel_request` tool and one-turn authorization contract.

Local protocol 430 is reserved for the changed catalog/tool surface. Its catalog
revision is hash-pinned; protocol shape fixtures and the shared client version
are updated. Relay protocol remains 70: peer and persisted legacy structures
are retained for decoding/settlement, with no new peer field or authority path.
Clients consume the shared catalog, so removing an entry does not require raising
native or web minimum versions. Old clients sending `/meta` receive the same
kernel refusal. Trusted internal legacy task settlement is not a new exposed mode.

Validation entry points:

```sh
node --test apps/cli/scripts/meta-retirement.test.mjs
cargo test -p chariox-kernel --lib retired_meta -- --nocapture
cargo test -p chariox-kernel --lib runtime_mcp_retires_meta_tools -- --nocapture
cargo test -p chariox-kernel --lib local::api::tests::protocol_shapes -- --nocapture
```

The serialized-catalog drill crosses the request/router/serialized-response
boundary and asserts protocol 430, the pinned catalog, sudo availability, and
Meta absence. The prompt drill verifies refusal with no active turn, sudo record,
or passkey popup. Public Meta tool tests/specs are removed with their retired
surface; common workflow and sudo coverage remains. Linux fixture tests do not
establish standalone Web/native or managed-machine acceptance, or independent
MP-11 security review. Evidence lives outside Git under the kasudo evidence root.
