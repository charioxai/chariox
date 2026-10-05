# MP-08 / MP-11 — held Meta retirement (PR 9)

The owner approved removal on 2026-10-05, with a shipping gate: retire `/meta`
only at the end of the program after the coordinator confirms `/sudo` is
validated. This branch stays on HOLD until then. It removes
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
kernel refusal.

At startup, before listeners and restart recovery begin, the kernel aborts
unfinished legacy Meta tasks (active, paused or blocked), cancels queued Meta
tasks and the old Meta turn/notifications, clears Meta mode, and releases the
controlled agents. Completed/aborted task history and ordinary queued prompts,
controlled-agent work, and workflow queues remain. Cancellation IDs are recorded
in the existing durable session update, and a checkpoint commits modes and
prompt settlement. A write failure fails bootstrap; the next restart retries.
There is no automatic conversion into sudo. Trusted internal legacy settlement
is not a new exposed mode.

Validation entry points:

```sh
node --test apps/cli/scripts/meta-retirement.test.mjs
cargo test -p chariox-kernel --lib retired_meta -- --nocapture
cargo test -p chariox-kernel --lib runtime_mcp_retires_meta_tools -- --nocapture
cargo test -p chariox-kernel --lib retired_meta_restart -- --nocapture
node --test scripts/publication-dockerfile.test.mjs scripts/slice-relay-identity-contract.test.mjs
cargo test -p chariox-kernel --lib local::api::tests::protocol_shapes -- --nocapture
```

The serialized-catalog drill crosses the request/router/serialized-response
boundary and asserts protocol 430, the pinned catalog, sudo availability, and
Meta absence. Publication contracts check both image protocol defaults and
the runtime label against local 430, with relay 70 unchanged. Real app restart
regressions cover task status, agent mode, repeat-boot idempotence, subsequent
workflow queue progress, ordinary work preservation, and checkpoint failure/retry.
The prompt drill verifies refusal with no active turn, sudo record,
or passkey popup. Public Meta tool tests/specs are removed with their retired
surface; common workflow and sudo coverage remains. Linux fixture tests do not
establish standalone Web/native or managed-machine acceptance, or independent
MP-11 security review. Evidence lives outside Git under the kasudo evidence root.
