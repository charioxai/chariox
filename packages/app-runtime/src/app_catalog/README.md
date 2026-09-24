# App tools in the existing runtime MCP

This module is the verified catalog and invocation contract. It does not publish
tools, grant bindings, launch workers, implement human validation, or install an
additional MCP server. A schema-valid call is not an authorization decision.

`AppCatalog::compile` accepts the package verifier's private proof type, the exact
durable `StageTrustBinding`, and the corresponding enrolled publisher snapshot.
It retains the full signed release metadata and compiles input/output schemas
with the same offline Draft 7 compiler used by the package verifier. Tools retain
their declared action and protected effect routes for the kernel's common policy.

Names are `app_<up to 27 local-name characters>_<32 hex characters>`, at most 64
ASCII characters. The suffix is the first 128 bits of SHA-256 over a versioned
domain and length-prefixed full installation ID and local tool name. An update
keeps the name, another installation gets a different name, and different long
names with the same visible prefix remain distinct. Exact map lookup and a full
runtime-catalog collision check are mandatory. Names never encode authority;
App ID, installation, local name, generation and signed catalog digest remain
separate trusted metadata. Descriptions include the full readable identities.

`require_current`, `prepare`, and `ValidatedToolCall::accept` use a current
transaction on the kernel's existing database. They recheck the retained signer
binding, owner, enrollment revision/public key, active generation, admission
pause and complete signed release. Publisher revocation stops admission even
without an App update. Re-enrollment cannot silently revive an older catalog.
The supervisor must serialize policy/generation admission with enqueue and fence
in-flight broker work on changes; an old read transaction is not fresh admission.
Do not hold a database transaction across IPC or a human-validation interaction.

Requests carry the SDK's existing `tools.invoke` shape. The kernel supplies only
typed actor/Room/operation/task/turn identifiers, captured at submission; changing
the focus agent cannot retarget them. No arbitrary context, transcript, prompt,
credential or human-approval token is sent. Successful results are checked against
the declared output schema. Values are limited to 512 KiB, 32,768 nodes and 62
nested containers, with finite numbers and JavaScript-safe integers. Exact
serialized size is checked after the bounded tree walk. Stable catalog errors
contain no instance values; worker error payloads are bounded untrusted App data.

The call object owns one immutable request ID and is consumed on acceptance.
It checks response kind, correlation, generation and deadline. The single worker
actor must allocate IDs, enforce queue/in-flight bounds, handle cancellation and
late/duplicate responses, and route every tool, state, HTTP, lifecycle and event
operation over the same inherited channel. This module creates no channel reader.
Discarding a response cannot roll back an external effect.

## Remaining kernel integration

- `runtime/state/tool_dispatch.rs`: add App specs and dispatch alongside dynamic
  script/connector tools after resolving the existing runtime MCP auth token to
  one authoritative provider run. Use `transport/runtime_tools::RuntimeToolSpec`
  and `RuntimeToolResult`; keep `router/runtime_tool_bridge.rs` as wiring. Check
  the combined namespace before publishing any colliding catalog. The current
  Meta-agent early return also needs an explicit App path; do not accidentally
  make App use depend on agent presentation mode.
- `extension.rs`, `agent_config_runtime_state.rs`, and
  `tool_dispatch/extension_request_tool.rs`: extend the existing grant kind with
  App and use installation ID as its stable name. The user selection, trusted
  foreground intent and policy-allowed self-grant all use the same mutation and
  provider catalog-refresh path. Existing effective YOLO/Ask policy and runtime
  interactions govern routine access; the common effect policy handles critical
  validation through every UI/tool/automation route. Binding selects the agent's
  catalog and does not restrict its ability to operate an accessible App view.
- Capture focus/session/provider/turn authority on submission in kernel state.
  A foreground App never starts a workflow by itself. Explicit revocation must
  suppress automatic regrant; closing a view need not revoke an in-flight task.
  Keep pending provider refresh visible instead of claiming immediate readiness.
- `extension.rs` remote manifests, `home_extension_authorizer.rs` and the existing
  home proxy dispatch must carry the App identity and current generation/catalog
  revision through the existing home/worker authority checks. A stale projected
  manifest cannot authorize a call. Provider-specific name aliases belong to
  the normal trusted adapter, with exact canonical lookup afterward.
- The serialized App extension kind and public binding commands require a shared
  protocol bump, snapshot tests and local/relay drills. They are not changed here.

Tests use signed tiny packages and an in-memory kernel-schema database. They
exercise metadata/schema/provenance logic, not a running App, cancellation,
external-effect safety, the complete binding path or provider catalog refresh.
