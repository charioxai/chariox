# M9 Workflow Publication Cloud Deployment Plan

This plan extends workflow publication from local serving and relay-display
embedding into product-shaped Cloud deployments. It intentionally covers both v1
deployment modes:

- public Cloud ingress with execution on the user's local machine
- public Cloud ingress with execution in one hosted Docker container per
  deployment

The current local publication runtime, exported package format, transports,
trace fanout, human HTTP split viewer, and publication container image are the
starting point. This plan does not replace `chariox serve`; it adds Cloud
deployment control, Hetzner runtime ingress, and hosted container lifecycle.

## Product Boundary

Chariox Cloud on Scalingo remains the control plane:

- account authentication
- deployment records
- runner registration
- package upload metadata
- deployment commands
- status/log metadata
- web terminal UI

The public publication endpoint is not served by the Scalingo API/web process.
Runtime traffic goes through a dedicated publication ingress on the Hetzner
runtime host:

```text
caller
  -> Hetzner publication ingress
  -> local-runtime connector or hosted publication container
  -> publication gateway
  -> kernel-owned publication runtime
  -> provider CLI/adapter
```

This keeps SSE, WebSocket, MCP, artifact uploads, and long-running human HTTP
viewers off the Scalingo control-plane app. Scalingo can link to and manage the
endpoint, but it must not become the runtime proxy.

## Existing Baseline

Already implemented in OSS:

- `workflow publication create|list|show|export|config|disable`
- portable publication package export
- `chariox serve <package> <port>` local gateway launcher
- package materialization into hidden, non-editable publication runtime sessions
- provider/model binding validation and local replacement prompts
- extension/credential requirements file validation
- transports: `human_http`, `api_sse_json`, `websocket_json`, `mcp`
- human HTTP split viewer with output pane, trace pane, attachment form, and
  renderable HTML final output
- per-node trace exposure policy with `output_summary`, `assistant_messages`,
  `thinking`, and `tool_use`
- relay display tunnel registration for served publication URLs
- Docker publication image that runs kernel plus gateway in one container
- local container drill for deterministic/dev-stub publications
- Cloud web terminal Published Workflows side panel and central iframe embedding
  for currently served publication URLs

Missing for this plan:

- durable Cloud publication deployment records
- Hetzner publication ingress service
- Hetzner publication runner service
- stable public deployment URLs
- Cloud-to-runner deployment job API
- package upload/download path for runner deployment
- hosted container lifecycle in Cloud
- local-runtime connector registration against the new ingress
- real-provider hosted-container drills against Codex, Claude Code, and OpenCode
- general arbitrary-user provider credential onboarding

## Deployment Modes

### Local Runtime Ingress

The workflow runs on the user's local machine. The public URL is hosted by the
Hetzner publication ingress.

```text
caller
  -> Hetzner publication ingress
  -> local serve connector over outbound tunnel
  -> local publication gateway
  -> local kernel
  -> local provider credentials
```

User flow:

```bash
chariox publication deploy <package> --mode local-runtime
chariox serve <package> <port> --cloud-deployment <deployment-id>
```

`chariox serve` still owns local gateway startup and local materialization. The
new flag only connects the served gateway to the Cloud deployment and registers
the current backend target with the publication ingress.

### Hosted Container

The workflow runs in one Docker container on the Hetzner publication runner.

```text
caller
  -> Hetzner publication ingress
  -> hosted publication container
  -> kernel inside container
  -> mounted provider credentials
```

User flow:

```bash
chariox publication deploy <package> --mode hosted-container
```

The runner starts one container per deployment using the existing publication
runtime image. The publication package is mounted read-only at `/publication`,
and the workspace/runtime state is mounted from a deployment-private directory.

## Public URL Contract

For staging, use the Hetzner publication ingress host. Do not route public
publication traffic through the Scalingo app.

```text
https://<publication-ingress-host>/<slug>/
https://<publication-ingress-host>/<slug>/<prompt>
```

Later product DNS can map the same contract to:

```text
https://<slug>.chariox.run/
https://<slug>.chariox.run/<prompt>
```

Code and protocol fields should call this `public_base_url`, not encode a
domain-specific assumption.

Supported transport paths under `public_base_url`:

- human HTTP root form: `GET /`
- human HTTP prompt URL: `GET /<prompt>`
- API SSE JSON: `POST /invoke`
- WebSocket JSON: `/.well-known/chariox/publication/ws`
- MCP: `POST /mcp`
- status: `GET /.well-known/chariox/publication/status`

Ingress must preserve streaming and must not buffer SSE/WebSocket output.

## Cloud Deployment Record

Add Cloud persistence for publication deployments.

Required fields:

- `id`
- `account_id`
- `created_by_user_id`
- `mode`: `local_runtime` | `hosted_container`
- `slug`
- `public_base_url`
- `status`: `pending` | `package_uploaded` | `starting` | `ready` |
  `unavailable` | `failed` | `stopped`
- `publication_id`
- `publication_alias`
- `workflow_id`
- `endpoint_id`
- `hook_id`
- `transport`
- `package_digest`
- `package_version`
- `runner_id`
- `backend_target`
- `runtime_session_id`
- `credential_profile`: nullable, initially `miguel_staging` for hosted real
  provider validation
- `last_health_at`
- `last_error`
- timestamps

`backend_target` is owned by the runtime side:

- local runtime: active connector/tunnel id
- hosted container: container id and local runner port

The record stores operational routing metadata only. It must not store provider
auth material or Chariox user session tokens.

## Runner Service

Add `chariox-publication-runner` for the existing Hetzner runtime machine.

Responsibilities:

- authenticate to Cloud with a scoped runner token
- poll or subscribe for deployment jobs
- download publication packages
- create deployment directories
- start one Docker container per hosted-container deployment
- mount package/workspace/runtime volumes
- expose container port to the ingress routing table
- report health, logs, and lifecycle state to Cloud
- stop/restart/remove containers
- clean stale containers, temp dirs, and images

The runner is not a workflow authority. It owns Docker lifecycle only.

Hosted container command shape:

```bash
docker run --rm \
  --name chariox-publication-<deployment-id> \
  -v <package-dir>:/publication:ro \
  -v <workspace-dir>:/workspace \
  -v <runtime-home-dir>:/home/chariox \
  -e CHARIOX_PUBLICATION_PACKAGE=/publication \
  -e HOST=0.0.0.0 \
  -e PORT=3000 \
  chariox-publication:providers-all standalone
```

During the first validation phase, use a staging credential profile on the
runner to mount or inject the user's existing provider credentials. Those
credentials must remain outside images and publication packages.

## Publication Ingress

Run the ingress on the Hetzner runtime host. It can be a small Chariox service or
Caddy/Traefik plus a dynamic routing adapter.

Responsibilities:

- terminate public HTTPS for the staging publication host
- route by deployment slug
- forward HTTP, SSE, WebSocket, and MCP to the active backend target
- serve clear unavailable responses when no backend is connected
- preserve request path, method, headers needed by the gateway, request body,
  and streaming responses
- expose minimal ingress health and routing diagnostics to the runner/Cloud

Ingress routes to either:

- a local-runtime connector connection
- a hosted container local port

The external caller should not need to know which backend is used.

## Local Runtime Connector

Extend `chariox serve` with Cloud deployment registration:

```bash
chariox serve <package> <port> --cloud-deployment <deployment-id>
```

The connector path should:

- start the existing local publication gateway
- keep local kernel/provider credential behavior unchanged
- authenticate to Cloud with the user's local Cloud session
- register the active local backend with the Hetzner publication ingress
- keep a heartbeat and reconnect loop
- unregister or mark unavailable on shutdown
- allow the same public URL to resume after local reconnect

If the local machine is offline, human HTTP returns an unavailable page, API SSE
returns a structured unavailable event/error, WebSocket closes with an
unavailable reason, and MCP returns a structured unavailable response.

## Provider Credentials For This Phase

This plan validates real providers now, but defers arbitrary-user provider login
UX until after the deployment pipeline works.

Rules:

- provider credentials are never baked into images
- provider credentials are never included in publication packages
- Chariox Cloud user account/session credentials are never included in images or
  packages
- hosted containers receive only scoped deployment/runtime identity plus
  staging provider credential mounts/env needed for validation
- local-runtime deployments use the local user's existing provider credentials

Initial hosted-container validation uses a staging credential profile:

```text
credential_profile = miguel_staging
```

That profile is runner-local/Cloud-staging-only and maps to the existing provider
credential material needed to run Codex, Claude Code, and OpenCode. It must be
clearly marked as temporary validation plumbing.

Deferred product credential onboarding:

- Codex: device auth or API-key/access-token setup, validated by provider login
  status
- Claude Code: `CLAUDE_CODE_OAUTH_TOKEN`/API-key setup for headless runtime
- OpenCode: provider auth/config persisted in deployment volume

Do not implement generic provider auth UX before the ingress/container pipeline
and real-provider drills pass.

## Cloud API

Add control-plane endpoints for users:

```text
POST /publication-deployments
GET /publication-deployments
GET /publication-deployments/:id
POST /publication-deployments/:id/package
POST /publication-deployments/:id/start
POST /publication-deployments/:id/stop
POST /publication-deployments/:id/restart
GET /publication-deployments/:id/logs
```

Add runner-facing endpoints:

```text
GET /runner/publication-jobs
POST /runner/publication-jobs/:id/status
POST /runner/publication-deployments/:id/heartbeat
POST /runner/publication-deployments/:id/logs
POST /runner/publication-deployments/:id/backend-target
```

Runner endpoints require scoped runner tokens. Deployment/container tokens must
not grant general user account privileges.

## CLI

Add deployment commands without changing existing publication authoring commands:

```bash
chariox publication deploy <package-dir|publication.json> --mode local-runtime
chariox publication deploy <package-dir|publication.json> --mode hosted-container
chariox publication deployments list
chariox publication deployments show <deployment-ref>
chariox publication deployments logs <deployment-ref>
chariox publication deployments stop <deployment-ref>
chariox publication deployments restart <deployment-ref>
```

`workflow publication create|export|config` remains the authoring/package path.
The deployment commands target Cloud deployment records.

## Cloud Web UI

Extend the Published Workflows side panel into a publication/deployment surface.

Required display:

- package/publication identity
- deployment mode
- public URL
- transport
- status
- runner/backend target
- credential profile or credential state
- latest health
- latest error
- latest run/output
- watchdog status when present

Required actions:

- open public endpoint embedded in central panel
- copy public URL
- view logs
- stop/restart hosted container deployment
- show local-runtime serve command for local execution deployments

The central panel should embed `public_base_url` for browser-compatible
transports. Cloud should not separately render publication traces; the embedded
publication viewer owns output and trace display.

## Implementation Phases

### Phase 1: Documentation And Schema

- Land this plan.
- Update architecture/protocol docs.
- Add Cloud deployment schema and repository tests.
- Add shared API contract types.

### Phase 2: Runner Skeleton And Control Plane

- Add runner token model.
- Add runner job polling/status APIs.
- Implement `chariox-publication-runner` heartbeat and job loop.
- Validate against the existing Hetzner machine without starting containers.

### Phase 3: Hosted Container No-Provider Pipeline

- Upload/export package to Cloud.
- Runner downloads package.
- Runner starts one container per deployment with deterministic/dev-stub
  workflow package.
- Ingress routes `public_base_url` to the container.
- Cloud UI shows ready/status/logs.

### Phase 4: Local Runtime Ingress Pipeline

- Add deployment creation for `local_runtime`.
- Extend `chariox serve` with `--cloud-deployment`.
- Register local backend with Hetzner ingress.
- Validate public URL while execution remains local.
- Validate disconnect/reconnect unavailable behavior.

### Phase 5: Hosted Container Real Providers With Staging Credentials

- Build/use provider-capable publication runtime image.
- Mount/inject `miguel_staging` provider credential profile on the Hetzner
  runner.
- Validate hosted-container workflows with real Codex, Claude Code, and OpenCode.
- Keep credentials out of image/package/logs.

### Phase 6: Full Transport Matrix

Run the full matrix for both deployment modes and all real providers where the
transport applies:

- human HTTP prompt URL
- human HTTP root form
- attachment upload
- split viewer output pane
- trace pane with per-node `output_summary`, `assistant_messages`, `thinking`,
  and `tool_use`
- final HTML dashboard rendering in sandboxed iframe
- API SSE `queued`, `started`, `partial`, `trace`, `final`, `error`
- WebSocket `ready`, `accepted`, `queued`, `started`, `partial`, `trace`,
  `final`, `error`
- MCP tool list/call final output
- watchdog scheduled publication status/output
- stop/restart
- unavailable backend behavior
- Cloud central-panel embedding

Real-provider visual drill prompt:

```text
Generate a vibrant dashboard as a compact self-contained HTML document.
The dashboard must visibly include the title text `Real Provider Workflow Dashboard`.
The main dashboard element must include `data-chariox-real-provider-dashboard="true"`.
Submit final workflow output as {"kind":"html","html":"<full html document>"}.
```

Required evidence:

- browser screenshots for local-runtime human HTTP dashboard
- browser screenshots for hosted-container human HTTP dashboard
- Cloud web terminal screenshots embedding both modes
- API SSE event transcript files
- WebSocket event transcript files
- MCP response files
- runner logs with secrets redacted
- Cloud deployment state snapshots

### Phase 7: Product Credential Onboarding

After phases 1-6 pass, add arbitrary-user provider auth setup:

- provider-specific setup flows
- Cloud secret storage
- deployment credential state UI
- provider auth revocation/rotation
- preflight blocks for missing credentials

This phase is explicitly deferred until the runtime deployment pipeline is
validated with real providers using staging credentials.

## Done Criteria

The feature is not complete until:

- both deployment modes work through the Hetzner publication ingress
- Scalingo Cloud app is control plane only for runtime traffic
- hosted containers run one container per deployment
- local-runtime deployments execute on the local machine through public ingress
- all four transports work from the public URL
- watchdog publications work in hosted container mode and appear in Cloud
- real Codex, Claude Code, and OpenCode provider drills pass in hosted container
  mode using staging credentials
- real-provider local-runtime drills pass using local credentials
- final HTML dashboard rendering is visually verified in local browser and Cloud
  central panel
- credentials are absent from images, packages, logs, and Cloud deployment
  metadata
- cleanup removes drill containers, temp dirs, images, and stale runner state
