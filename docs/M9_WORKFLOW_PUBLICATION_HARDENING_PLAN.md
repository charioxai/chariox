# M9 Workflow Publication Hardening Plan

This document records the remaining hardening work after the first M9 workflow
publication implementation and validation pass. The current implementation has
local publication support and a Cloud Published Workflows tab, but not every
transport/deployment permutation has been validated end to end.

## 1. Current Baseline

The current validation baseline proves:

- portable publication packages are exported without relay token material
- `chariox serve` materializes hidden publication runtime sessions
- hidden publication runtime sessions are excluded from normal session lists
- missing skills and credentials fail before the publication gateway listens
- provider/model replacement prompts run through real `chariox serve` stdin and
  persist replacements to `bindings.local.json`
- `human_http` works locally from a browser URL
- the `human_http` root page supports prompt plus artifact upload
- the `human_http` status page renders running and completed states
- `api_sse_json` locally streams queued, started, partial, and final events
- `websocket_json` locally accepts JSON invocation and artifact chunks
- `mcp` locally exposes a published workflow tool and accepts invocation
- watchdog publications can be exported, served locally, and scheduled
- Cloud has a dedicated Published Workflows side-panel tab
- Cloud can open a local-only published workflow through a relay display tunnel

This baseline does not yet prove every partial/final output path for every
transport both locally and through Cloud.

## 1.1 Trace Fanout And Embedded Viewer Extension

The next hardening slice extends the publication contract beyond partial/final
outputs:

- a rapid repeated invocation drill must prove one published runtime can accept
  a second prompt immediately after a first workflow run reaches a terminal
  state
- publication packages and kernel-owned publication records carry a
  per-node `trace_exposure` policy
- trace levels are `output_summary`, `assistant_messages`, `thinking`, and
  `tool_use`
- trace events are exposed only when the node-specific policy allows the level
- `human_http`, `api_sse_json`, and `websocket_json` forward `trace` events
  using their native event mechanisms
- `mcp` remains final-output first; trace/progress mapping is optional until an
  MCP client surface consumes it cleanly
- the `human_http` page becomes a split viewer with output/status on the left
  and exposed traces on the right
- final output shaped as `{ "kind": "html", "html": "..." }` is rendered in a
  sandboxed iframe in the left pane
- Cloud opens `human_http` publications in the central terminal panel by
  embedding the relay display URL; the embedded HTML owns output and traces

## 2. Transport Output Contract

Each transport should have an explicit output contract.

### `human_http`

- `GET /<prompt>` enqueues an invocation.
- Browser requests receive an HTML page.
- The HTML page follows an SSE stream through queued, running, partial, final,
  and error states.
- When enabled by publication policy, the HTML page also renders `trace` events
  in a right-side pane tagged by node or agent alias.
- Renderable HTML final output replaces the left output pane with a sandboxed
  iframe.
- `GET /` renders a prompt and artifact upload form.

### `api_sse_json`

- `POST /invoke` accepts JSON input only.
- Artifact inputs use JSON payloads, including base64 data and URL refs where
  supported.
- The response is an SSE stream with queued, started, partial, final, and error
  events.
- The response also includes `trace` events when the publication policy exposes
  them.
- Final events carry the normalized workflow output envelope.

### `websocket_json`

- The WebSocket protocol accepts JSON messages.
- Artifact upload uses begin, chunk, and end messages.
- Invocation emits accepted, queued, started, partial, final, and error
  messages.
- Invocation emits `trace` messages when the publication policy exposes them.
- Final messages carry the normalized workflow output envelope.

### `mcp`

- Published workflow hooks appear as MCP tools.
- Tool invocation returns the deterministic final workflow output and artifact
  refs.
- MCP partial/progress streaming should be implemented only if it maps cleanly
  onto MCP progress/resource conventions. Otherwise MCP is final-output only for
  v1 and the limitation must be documented.

## 3. Local Transport Hardening

### `human_http`

- Keep the existing browser drill.
- Add explicit assertions that the SSE stream observes queued, running,
  partial, final, and error events.
- Keep screenshot verification for the final HTML status page.
- Keep root-page prompt and artifact upload coverage.
- Verify the split viewer renders the trace pane and tags trace entries by
  node/agent alias when traces are enabled.
- Verify renderable HTML final output replaces the left output pane with a
  sandboxed iframe and leaves the trace pane visible.

### `api_sse_json`

- Keep the queued, started, partial, and final event assertions.
- Add artifact input coverage for base64 payloads.
- Add URL artifact ref coverage if URL refs are supported by the parser.
- Assert final output body shape, not only event names.
- Add `trace` event assertions for enabled nodes and absence assertions for
  disabled nodes/levels.

### `websocket_json`

- Extend the local WebSocket drill to require:
  - artifact begin/chunk/end acknowledgements
  - invocation accepted metadata
  - queued and started messages
  - partial output message
  - trace messages when the publication policy enables them
  - final output message
  - structured error on invalid input

### `mcp`

- Strengthen the MCP drill to require:
  - tool list includes the published workflow tool
  - invocation returns deterministic final output
  - artifact refs are returned when the workflow output includes artifacts
- Decide whether MCP progress is part of v1 or explicitly deferred.

## 4. Watchdog Publication Hardening

Watchdog endpoints do not have an external request trigger, but they still need
publication lifecycle coverage.

- Verify watchdog publications appear in publication inventory.
- Verify the Published Workflows Cloud tab lists watchdog publications.
- Verify serve materializes the hidden runtime session.
- Verify scheduled runs attach to the correct endpoint and queue.
- Add a status/output surface that shows:
  - last scheduled run
  - run status
  - last final output
  - artifact refs, if any
- For v1, watchdog Cloud support can be read-only status/output. It does not
  need an invoke form.

## 5. Cloud Published Workflows Tab

The Cloud tab should expose publication status without treating publication
runtime sessions as normal editable sessions.

Required UI state:

- publication id and alias
- workflow id/name when available
- endpoint id/name when available
- transport
- route and HTTP methods where applicable
- queue binding
- runtime status
- runtime location
- deployment/open URL
- last run status when available

Required actions:

- `open`: open the served or tunnel URL when available
- `start`: local instruction for v1, future remote command
- `stop`: local instruction for v1, future remote command
- `invoke`: enabled only when the transport has a Cloud-compatible tunnel
  invocation path
- `configure`: local package command for v1
- `logs`: future unless a log surface is wired

The browser drill should assert:

- Published Workflows appears as its own side-panel tab
- regular publications are listed
- watchdog publications are listed distinctly
- local-only publications without URLs show the correct disabled/manual state
- relay display tunnel URLs open through the Cloud central embedded view for
  `human_http`

## 6. Cloud Tunnel Invocation Matrix

Cloud tunnel validation is the main missing area.

### `human_http`

- Open the publication from the Published Workflows tab.
- Render the split HTML viewer through the relay display tunnel.
- Verify queued, running, partial, and final states.
- Verify exposed traces appear in the right pane when configured.
- Capture browser screenshots of the final page and generated HTML iframe.

### `api_sse_json`

- Define the tunnel invocation path:
  - browser-side fetch through a relay display/proxy endpoint, or
  - Cloud action that asks the kernel to call the local publication on behalf of
    the browser.
- Drill queued, started, partial, final, and error events through that tunnel.

### `websocket_json`

- Add relay tunnel support for WebSocket publication endpoints if missing.
- Open a WebSocket through the tunnel URL.
- Send artifact chunks.
- Invoke the workflow.
- Assert partial and final messages.

### `mcp`

- Cloud does not need to invoke MCP directly in v1.
- Cloud should list MCP publications and show endpoint/deployment metadata.
- MCP invocation should remain validated with MCP clients unless a Cloud MCP
  client surface is deliberately added.

## 7. Container Deployment Milestone

Container deployment is future work and should not be considered complete in the
current M9 implementation.

### Package Inputs

The container build input should include:

- `publication.json`
- `workflow.snapshot.json`
- `requirements.json`
- bundled scripts and local APIs used by the workflow
- local binding configuration or provider/model override configuration
- no secrets

### Runtime Image

The image should contain:

- Chariox kernel
- publication gateway/server
- required runtime assets
- provider CLI dependencies where practical

Secrets and provider credentials must be injected at deploy time, not baked into
the image.

### Container Validation

- Build the image locally.
- Run the container with environment-injected credentials.
- Serve one `human_http` publication.
- Serve one `api_sse_json` publication.
- Verify hidden runtime materialization.
- Verify output streaming.
- Verify missing requirements fail before listen.

### Hosted Deployment

Chariox Cloud can later host the container as a generic web service. In that mode
Cloud should expose URL/IP/logs and deployment lifecycle controls, while the
published workflow remains independently reachable by non-Chariox users if the
owner configures it that way.

## 8. Execution Order

1. Separate the validation matrix into local, Cloud tab, Cloud tunnel, watchdog,
   and container sections.
2. Add missing local transport assertions for `human_http`, `websocket_json`,
   and `mcp`.
3. Add watchdog inventory and status/output validation.
4. Extend the Cloud browser drill for Published Workflows tab completeness and
   `human_http` tunnel completion.
5. Add Cloud tunnel invocation for `api_sse_json`.
6. Add Cloud tunnel invocation for `websocket_json`.
7. Decide and document MCP Cloud behavior.
8. Start the container milestone only after the local and Cloud tunnel matrix is
   green.

## 9. Definition Of Done

The publication model should not be called fully complete until this matrix is
green:

| Capability | Local | Cloud Tab | Cloud Tunnel |
| --- | --- | --- | --- |
| `human_http` GET plus HTML/SSE | yes | yes | yes |
| `human_http` upload form | yes | yes | yes |
| `api_sse_json` queued/partial/final | yes | listed | yes |
| `websocket_json` artifacts/partial/final | yes | listed | yes |
| `mcp` final output/artifact refs | yes | listed | optional |
| watchdog scheduled publication | yes | yes | status/output yes |
| provider/model override | yes | n/a | n/a |
| missing requirements fail before listen | yes | n/a | n/a |
| hidden runtime sessions | yes | yes, not normal sessions | yes |
| rapid repeated invocation | yes | yes | yes |
| trace fanout per node/level | yes | listed | yes |
| split human HTTP viewer | yes | yes | yes |
| renderable HTML final output | yes | yes | yes |
| container deployment | future | future | future |
