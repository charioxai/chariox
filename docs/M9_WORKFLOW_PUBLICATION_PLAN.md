# M9 Workflow Publication Plan

M9 makes a workflow runnable outside the interactive authoring session. A
published workflow is not a live pointer to an editable session. It is a
portable publication package plus a kernel-owned runtime that is materialized
when the publication is served or deployed.

The first implementation target is one transport, `human_http`, with HTTP GET
address-bar invocation and an HTML/SSE status page. The remaining transports
come after the package/runtime model is validated end to end.

## M9.0 Reset and Cleanup

Before adding the new model, remove the old active publication surfaces that do
not belong to v1.

Delete from the active publication product/code path:

- built-in publication auth modes and paired-sender auth
- Slack, Telegram, Discord, WhatsApp, and Signal publication connectors
- connector-specific publication drills and docs that treat those connectors as
  v1 publication transports
- session-bound publication behavior where a gateway invokes an editable source
  session as the runtime authority

Keep only reusable low-level code when it directly supports the new transport
model. If a removed feature is needed later, recover it intentionally as a new
hook type over the published-workflow runtime.

## M9.1 Core Concepts

### Published Workflow

A published workflow is the deployable/runtime unit. It contains:

- a workflow snapshot
- workflow endpoints
- workflow queues
- agent/provider/model/permission settings captured from existing concepts
- extension requirements
- packaged scripts and editable app assets when present
- one or more hooks

The published workflow owns one materialized runtime session while served. That
runtime is hidden, non-editable, and excluded from normal local/web CLI session
lists. It should be inspectable only through publication-specific logs, runs,
status, and stop/start surfaces.

### Hook

A hook is an access surface into a published workflow runtime. A hook binds:

- a transport
- one workflow endpoint
- one workflow queue
- parser/input rules
- response/event mode

Multiple hooks may feed the same published workflow runtime, endpoints, and
queues. This is required so queue priorities continue to compete inside one
workflow runtime.

### Publication Package

Publishing creates an external program/package that can be shared and edited.
The package should be a normal directory, not only a kernel database record.

Target shape:

```text
published-workflow/
  publication.json
  workflow.snapshot.json
  requirements.json
  bindings.example.json
  public/
    index.html
    app.js
    styles.css
  scripts/
  run.sh
  README.md
```

`publication.json` describes hooks, default host/port behavior, package
version, and generated app assets. `workflow.snapshot.json` stores the captured
workflow/session-derived runtime state using existing Arroba concepts.
`requirements.json` lists required extensions and credentials. Local provider
and model substitutions are stored outside the package in a local binding file.

The generated app/server is editable and distributable. It owns transport and
client interaction only. The kernel remains the authority for workflow
scheduling, queues, agents, provider runs, outputs, artifacts, and state.

## M9.2 Serve Lifecycle

`arroba serve <publication-package-or-ref> <port>` starts a published workflow.

Serve sequence:

1. Load the publication package.
2. Resolve provider/model availability. If the captured provider/model is not
   available, prompt on stdin for a replacement from providers/models known to
   the kernel, then persist the answer in local publication bindings.
3. Verify extension and credential requirements.
4. Materialize a hidden publication runtime session from the workflow snapshot.
5. Start agents/provider runs/extensions/slices required by that runtime.
6. Start the selected transport server/listener.
7. Mark the published workflow ready only after validation and listener startup
   succeed.

Published runtime sessions:

- are kernel-owned
- are hidden from normal session lists and side panels
- are non-editable through ordinary workflow/session commands
- use the workflow queues captured in the publication snapshot
- accept prompts only through published hooks or publication management
  commands

There is no workflow-run concurrency within one published workflow runtime. The
existing workflow queue system decides when each prompt runs.

## M9.3 Invocation Envelope

An invocation envelope is created at call time, after transport parsing and
before enqueueing into the published runtime.

Logical shape:

```json
{
  "publication_id": "pub_...",
  "hook_id": "hook_...",
  "invocation_id": "inv_...",
  "transport": "human_http",
  "endpoint_id": "endpoint_...",
  "queue_ref": "default",
  "input": {
    "prompt": "..."
  },
  "artifacts": [],
  "mode": "stream"
}
```

This envelope should become a kernel-native structured invocation. It must not
remain only a JSON string hidden inside the prompt compatibility path.

## M9.4 Transports

V1 transport set:

- `human_http`
- `api_sse_json`
- `websocket_json`
- `mcp`

Do not expose all transports by default. A hook chooses one transport. A
published workflow can have several hooks.

### `human_http`

For a human using a browser:

```text
GET /<url-encoded-prompt>
```

Behavior:

- parse the prompt from the path with a regex parser that can URL-decode named
  captures
- enqueue the prompt into the hook's configured workflow queue
- return an HTML status page
- the page opens an SSE stream to show queued/running/partial/final output

Root behavior:

```text
GET /
```

returns an editable HTML input form. The form can accept text and file uploads,
then transition into the same output/status page.

### `api_sse_json`

For scripts and applications:

```http
POST /invoke
Accept: text/event-stream
Content-Type: application/json
```

The request body is JSON. Artifacts are represented as JSON, either as base64
content or remote URLs.

The response is an SSE stream:

```text
event: queued
data: {"invocation_id":"inv_..."}

event: started
data: {"workflow_run_id":"run_..."}

event: partial
data: {"message":"..."}

event: final
data: {"message":"...","artifacts":[]}
```

Polling is not part of the primary v1 API shape.

### `websocket_json`

For bidirectional live clients. Messages are JSON.

Artifact upload can happen over WebSocket using base64 chunks:

```json
{ "type": "artifact.begin", "name": "image.png", "media_type": "image/png" }
{ "type": "artifact.chunk", "artifact_id": "art_...", "data_base64": "..." }
{ "type": "artifact.end", "artifact_id": "art_..." }
```

Invocation:

```json
{
  "type": "invoke",
  "input": { "prompt": "describe this image" },
  "artifacts": [{ "id": "art_..." }]
}
```

### `mcp`

A published workflow can be exposed as an MCP tool. The tool invocation enters
the same published runtime and queue system. Progress should use MCP progress
notifications where the client supports them, and final output should include
message plus resource/artifact references.

## M9.5 Artifacts

Inbound artifacts:

- `human_http`: root form supports upload; address-bar GET is prompt-only
- `api_sse_json`: JSON base64 artifacts or URL references
- `websocket_json`: WebSocket JSON/base64 artifact chunks
- `mcp`: MCP content/resource mechanisms where available

Outbound artifacts:

- workflow outputs include message plus artifact references
- HTML pages render links/previews through the publication app
- API/WebSocket/MCP transports return artifact refs in final output events

Secrets must never be exported. Credentials required by MCPs/connectors/API
extensions are declared in `requirements.json` and resolved against the local or
hosted vault at serve/deploy time.

## M9.6 Requirements and Bindings

`requirements.json` lists extension requirements only. V1 does not attempt to
install missing remote dependencies automatically.

Example:

```json
{
  "mcps": [{ "name": "github" }],
  "skills": [{ "name": "ios-debugger-agent" }],
  "scripts": [{ "name": "summarize", "path": "scripts/summarize" }],
  "connectors": [{ "name": "linear" }],
  "credentials": [{ "name": "GITHUB_TOKEN", "used_by": "github" }]
}
```

At serve/deploy time the kernel must verify:

- required MCPs exist
- required skills exist
- packaged scripts are present and valid
- required connectors exist if referenced by granted extensions
- required credentials exist in the configured vault

Provider/model substitutions remain supported. If the exact captured
provider/model is unavailable, `arroba serve` prompts the user for a replacement
from available kernel providers/models and persists that choice in local
bindings. Additional commands must allow editing bindings per workflow node for
future runs.

## M9.7 Local, Remote, and Hosted Deployment

### Localhost

`arroba serve` binds `127.0.0.1` by default. Local callers access the published
transport directly.

Remote Arroba terminals and Arroba Cloud may call locally available published
workflows through a relay/kernel tunnel. The caller still uses the published
transport shape; the kernel relays the transport request and response on behalf
of the remote user.

### Remote Ingress, Local Runtime

A public ingress service exposes a URL and forwards requests over an outbound
publication tunnel to a local workflow runtime. The local machine keeps running
the kernel and published workflow. External callers should not be able to tell
whether the URL terminates at ingress-only hosting or a full hosted container.

This mode can be offered by Arroba Cloud or self-hosted by users, analogous to
self-hosted relay.

### Hosted Container

A container includes:

- kernel
- publication app/gateway
- workflow snapshot
- requirements manifest
- packaged scripts/assets
- startup config

It can run in Arroba Cloud or any user-managed environment. It should not
depend on the original user's machine being online.

### Access Policy

Access policy is separate from deployment mode:

- `personal`: only the owner can access through Arroba auth or local-only access
- `public`: no Arroba auth required; dangerous and must be explicit
- `authorized`: user-managed or future Arroba-managed team/user access

For v1 local serve, bind localhost by default. Public exposure requires an
explicit host/config choice and should warn loudly.

## M9.8 Web CLI and Cloud UX

Web CLI gets a dedicated side-panel tab:

```text
Published Workflows
```

It is not nested under the existing workflow tab.

The tab should show:

- published workflow name/ref
- hook transports
- status: stopped, starting, running, error
- deployment: localhost, cloud ingress, hosted container, self-hosted
- runtime location
- last invocation/run
- actions: start, stop, open, invoke, configure, logs

For v1, web terminal invocation of a local-only published workflow uses the
relay/kernel tunnel and renders the returned transport response. Browser drills
must verify that a `human_http` HTML status page opens and updates when invoked
from web terminal.

## M9.9 Watchdog Publications

Watchdog endpoints have no external request trigger, but they should be
publishable as scheduled hooks.

Scheduled publication behavior:

- publish captures the workflow snapshot and watchdog definition
- serve/deploy materializes the hidden runtime session
- the internal scheduler enqueues runs according to the watchdog policy
- the published workflow appears in the Published Workflows side-panel tab
- actions include start, stop, run now, logs, and outputs

Local and hosted-container watchdog publications are in scope after
`human_http`. Remote ingress for watchdog-only publications is unnecessary
unless the user also publishes a status/output UI hook.

## M9.10 Implementation Plan

### Phase 1: Cleanup

- Delete old publication auth and paired-sender code paths from the active v1
  publication surface.
- Delete Slack/Telegram/Discord/WhatsApp/Signal publication connector surfaces,
  commands, docs, and drills.
- Adjust tests so CI reflects the v1 transport set only.

### Phase 2: Package and Snapshot Model

- Define `publication.json`, `workflow.snapshot.json`, `requirements.json`, and
  local bindings format.
- Change publish/export to produce a durable package instead of a live
  session-bound gateway config.
- Add snapshot validation for endpoints, queues, agents, provider/model
  availability, and extension requirements.
- Add provider/model override prompts in `arroba serve`.
- Add publication configuration commands for per-node provider/model bindings.

### Phase 3: Publication Runtime Sessions

- Add a hidden/non-editable publication runtime session kind.
- Materialize runtime sessions from publication snapshots at serve time.
- Recreate workflow queues and agent/provider runs from the snapshot.
- Exclude publication runtime sessions from normal local/web CLI session lists.
- Expose publication-specific status, logs, runs, and stop/start commands.

### Phase 4: `human_http`

- Add `human_http` hook config.
- Extend regex parser with URL-decoding for named captures.
- Implement `GET /<prompt>` -> enqueue -> HTML/SSE status page.
- Implement `GET /` input/upload form.
- Implement SSE event stream for queued/running/partial/final output.
- Implement local `arroba serve <package-or-ref> <port>`.
- Add local end-to-end drill: publish, serve, open browser URL, verify HTML and
  SSE final output by screenshot.
- Add web-terminal tunnel drill: invoke local `human_http` publication from web
  terminal, render returned HTML/status page, verify with browser screenshot.

### Phase 5: `api_sse_json`

- Add `POST /invoke` JSON-only input.
- Support base64 and URL artifact refs.
- Stream SSE events until final output.
- Add script/curl drill proving queued/partial/final events.

### Phase 6: `websocket_json`

- Add JSON WebSocket protocol.
- Add WebSocket artifact begin/chunk/end messages.
- Add invocation, output, final, and error events.
- Add browser drill with file upload over WebSocket and screenshot verification.

### Phase 7: `mcp`

- Expose published workflow hooks as MCP tools.
- Map progress/final/artifacts onto MCP tool and resource concepts.
- Add MCP client drill proving invocation and final output.

### Phase 8: Deployment Extensions

- Add local-to-cloud/web-terminal tunnel support for published transports.
- Add Cloud Published Workflows tab and controls.
- Add remote ingress/local runtime design and drill.
- Add hosted-container packaging for Arroba Cloud and self-hosted deployment.

## M9.11 Validation Matrix

Required before considering the publication model complete:

- publish package is portable and contains no secrets
- `arroba serve` fails before listening when providers/extensions/credentials
  are missing
- provider/model overrides are prompted and persisted locally
- publication runtime sessions do not appear in normal session lists
- `human_http` works from local browser address bar
- `human_http` root page supports prompt plus artifact upload
- SSE page updates through queued/running/final states
- web-terminal tunnel can open and render a local-only published workflow
- API SSE streams queued/partial/final events
- WebSocket supports JSON invocation and artifact chunks
- MCP tool invocation returns final output/artifact refs
- watchdog publication starts scheduled runs without external trigger
- published workflow side-panel tab shows status/actions independently from the
  workflow authoring tab
