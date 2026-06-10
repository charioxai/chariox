# Agent Apps Launch-Ready Completion Plan

## Goal

Bring workflow publication and Agent Apps to launch-ready quality across the two supported v1 deployment modes:

- `local_runtime`: public Cloud ingress routes to a workflow gateway still executing on the user's local machine.
- `hosted_container`: Hetzner runner starts an isolated container with kernel, publication package, workflow gateway, provider CLIs, extensions, assets, and runtime state.

This plan completes the remaining work after the current prototype/hardening slices. It includes Codex, Claude, and OpenCode in live validation.

## Current Baseline

Already implemented and validated in automated tests:

- publication package upload/download route with a package-store abstraction
- dev local package store and fail-loud production package store placeholder
- lifecycle transition validation for start, stop, and restart
- hardened ingress proxying for body size, forwarded headers, sensitive header stripping, and response header filtering
- deployment observability fields in Cloud summaries
- deployment panel health, queue, replica, log, stop, restart, and reupload affordances
- public unmanaged access warnings in the web deploy modal
- CLI package reupload path
- Agent App config validation in the OSS gateway
- action-call hardening for external URLs, timeouts, response size, and audit logging
- Agent App runtime state persistence for overlays, persistent patches, invocation routes, and caller replica affinity
- Agent App queue/replica status endpoint
- runner enrichment of hosted-container backend targets with queue/replica status
- local-runtime backend registration with Agent App queue/replica status
- human viewer queue detail display

## Remaining Work

### 1. Durable Production Package Storage

Implement the production `PublicationPackageStore` backend.

Required behavior:

- keep `LocalFilePublicationPackageStore` only for development and tests
- add a real durable object-store implementation behind the existing interface
- require durable storage when Cloud runs in production/staging launch configuration
- reject startup or package upload loudly if durable package storage is not configured
- keep package URIs stable across Cloud API redeploys
- preserve package digest and version metadata
- support package reupload replacing the durable archive for the same deployment

Validation:

- upload package archive
- restart Cloud API process
- runner downloads the package after restart
- remove/archive-loss simulation returns a clear `package_missing` error
- CLI reupload restores the package
- restart deployment after reupload succeeds

### 2. Runner Startup Reconciliation

Make the Hetzner publication runner reconcile existing runtime state on startup.

Required behavior:

- discover existing `arroba-publication-*` containers
- read runner route files and per-deployment `runtime.json`
- probe each backend `/health`
- probe Agent App status when available
- re-register healthy deployments with Cloud
- mark missing or unhealthy deployments `unavailable`
- remove route entries pointing to dead containers
- preserve container logs before stopping or removing orphans
- avoid deleting containers owned by another runner or non-publication services

Validation:

- runner restart with healthy container preserves route and marks deployment ready
- runner restart with missing container marks deployment unavailable
- stale route is removed
- orphan publication container logs are preserved before cleanup
- unrelated containers are ignored

### 3. Credential Readiness

Surface provider credential readiness without storing or leaking credentials.

Required behavior:

- hosted runner checks the requested credential profile directory exists before container start
- container startup exposes provider CLI readiness checks for Codex, Claude, and OpenCode
- readiness check reports provider name, status, and stable error category
- credential contents are never logged
- Cloud deployment status includes credential readiness metadata
- Web deployments panel and CLI `show` display credential readiness

Error categories:

- `credential_profile_missing`
- `provider_cli_missing`
- `provider_auth_expired`
- `provider_auth_unknown`
- `provider_ready`

Validation:

- missing profile fails before container start with `credential_profile_missing`
- mounted profile starts container
- expired auth reports a non-secret error
- Codex readiness passes with staging profile
- Claude readiness passes with staging profile
- OpenCode readiness passes with staging profile

### 4. Web Reupload Wiring

Keep the CLI reupload path as the guaranteed recovery mechanism, then add Web reupload for deployments that still have a connected source kernel.

Required behavior:

- deployment records created from Cloud canvas retain enough source metadata to ask the connected kernel to re-export the package
- Web `Re-upload package` action detects whether source kernel/package source is available
- if available, Web asks the kernel to export a fresh package and uploads it to Cloud
- if unavailable, Web keeps the current explicit CLI recovery instruction
- successful reupload can optionally offer restart

Validation:

- Web reupload works for a deployment created from the connected canvas
- Web reupload shows CLI instruction for deployments with no reachable source kernel
- CLI reupload continues to work
- reupload followed by restart restores a missing package deployment

### 5. Lifecycle, Health, And Error Taxonomy

Finish the deployment lifecycle and health model.

Required behavior:

- standardize lifecycle statuses and valid transitions across API, runner, web, and CLI
- add stable `lastErrorCode` categories
- add health checks for:
  - package materialized
  - kernel reachable
  - publication gateway reachable
  - provider credentials ready
  - Agent App status reachable
  - ingress route reachable
- expose health check timestamp and check source
- append structured logs for lifecycle transitions and health failures

Error categories:

- `package_missing`
- `package_invalid`
- `container_start_failed`
- `credential_unavailable`
- `health_timeout`
- `gateway_unreachable`
- `ingress_unreachable`
- `provider_unavailable`
- `runner_unavailable`
- `unknown`

Validation:

- failed health check surfaces correct category
- Cloud UI and CLI display the same status/error fields
- restart from `failed` and `unavailable` works when the underlying issue is fixed
- invalid transitions are rejected with a clear error

### 6. Queue And Replica Hardening

Complete deployed workflow pool behavior.

Current state:

- queue/replica status is observable
- caller affinity survives gateway restart
- active leases and pending dispatch callbacks remain in-memory

Required v1 behavior:

- keep product-visible queue at the deployment/workflow pool level
- preserve per-caller ordering
- expose queue depth, active replicas, ready replicas, and queue age
- on gateway crash, fail in-flight HTTP/SSE/WebSocket calls clearly instead of silently resurrecting pending callbacks
- after restart, accept new calls using persisted caller affinity
- log timeout and queue-full events with request id and caller key hash

Validation:

- two callers with one busy replica produce queue status in viewer and Cloud UI
- per-caller ordering is preserved
- queued request dispatches after replica release
- gateway restart preserves caller affinity
- in-flight call interruption is logged and visible to the caller

### 7. Security And Ingress Final Pass

Finish the public ingress hardening matrix.

Required behavior:

- public unauthenticated access remains allowed but visibly intentional in Web and CLI
- ingress rejects oversized request bodies
- ingress strips hop-by-hop and sensitive internal headers
- ingress preserves encoded prompt path segments
- ingress preserves streaming behavior for SSE
- ingress preserves WebSocket close codes and reasons
- action proxy blocks external URLs by default
- non-local action URLs require explicit route policy opt-in

Validation:

- HTTP GET prompt path with encoded characters reaches the workflow unchanged
- API SSE streams through Cloud ingress
- WebSocket streams through Cloud ingress and close/error reasons survive
- oversized POST receives 413
- sensitive headers are not forwarded
- external action URL is rejected by default in a deployed container

### 8. Product UX And CLI Parity

Bring Cloud UI and CLI operator surfaces to the same baseline.

Web requirements:

- Workflow Deployments tab shows:
  - mode
  - transport
  - Agent App badge
  - route list
  - public URL
  - credential profile and readiness
  - health
  - queue depth
  - active/ready replicas
  - latest error code/message
  - latest structured log
- deploy modal validates:
  - slug
  - endpoint
  - transport
  - assets path
  - route path
  - replica count
  - credential profile
- central viewer embeds human HTTP deployments
- logs view is useful enough for operator diagnosis

CLI requirements:

- `arroba publication deployments list`
- `show`
- `logs`
- `stop`
- `restart`
- `reupload`
- status output matches Cloud UI fields
- public unmanaged access warning appears in deploy/start output

Validation:

- Web tests cover empty, ready, failed, local-runtime, hosted-container, and credential-error states
- CLI tests cover all deployment commands and output fields
- manual Cloud screenshots verify the operator flow

### 9. Live Provider Drills

Run live end-to-end drills with all supported provider CLIs:

- Codex
- Claude
- OpenCode

For each provider, run:

- local-runtime Agent App shopping flow
- hosted-container Agent App shopping flow

Each drill must validate:

- deploy from CLI
- deploy from Cloud canvas where supported
- public URL opens from browser
- browser URL prompt invokes workflow
- central Cloud viewer opens
- partial output appears
- trace panel shows configured levels
- final rendered app view appears
- overlay mutation works
- checkout action automation works
- deployment logs contain lifecycle and action events
- cleanup removes containers/processes/routes

Screenshots:

- save under `.artifacts/agent-apps-launch-hardening/`
- include provider, mode, transport, and timestamp in filenames

### 10. Container Drill Completion

Finish the hosted container drill as a repeatable launch gate.

Required behavior:

- package contains workflow snapshot, requirements, scripts/assets, trace policy, Agent App config, and provider CLI requirements
- package excludes provider credentials and Arroba Cloud credentials
- runner mounts explicit staging credential profiles for validation
- container starts kernel and gateway
- package materializes before serving
- public ingress URL reaches the container
- stop/restart are clean

Validation:

- Codex hosted container pass
- Claude hosted container pass
- OpenCode hosted container pass
- package missing plus reupload recovery pass
- restart persistence for overlays/persistent patches pass
- runner disk remains above configured free-space threshold after cleanup

### 11. Local-Runtime Drill Completion

Finish the local-runtime ingress drill as a repeatable launch gate.

Required behavior:

- local `arroba serve` equivalent starts publication gateway
- gateway registers Cloud backend with relay/display tunnel URL
- Cloud ingress reaches the local gateway through the kernel/relay path
- Cloud UI shows local serve readiness or recovery instruction
- unavailable local runtime is marked clearly

Validation:

- Codex local-runtime pass
- Claude local-runtime pass
- OpenCode local-runtime pass
- unavailable/recovery drill pass
- central Cloud viewer pass
- queue/trace/final output visible

### 12. Final Matrix

Automated:

- OSS server tests
- OSS CLI tests
- Cloud API tests
- Cloud worker tests
- Cloud web tests
- protocol/package shape tests if any serialized contract changes

Live:

- Codex local-runtime Agent App shopping flow
- Codex hosted-container Agent App shopping flow
- Claude local-runtime Agent App shopping flow
- Claude hosted-container Agent App shopping flow
- OpenCode local-runtime Agent App shopping flow
- OpenCode hosted-container Agent App shopping flow
- package archive missing and reupload recovery
- runner restart reconciliation
- gateway restart persistence
- replica queue and per-caller ordering
- oversized body and header-stripping ingress checks
- screenshots saved locally

Cleanup:

- stop all test deployments
- remove Hetzner publication containers
- prune stopped containers and safe build cache
- remove local temp dirs
- kill local test servers/provider processes
- verify local and Hetzner free disk thresholds

## Criteria For Done

- all automated tests pass
- all live provider drills pass for Codex, Claude, and OpenCode
- both deployment modes pass for all providers
- durable package storage survives Cloud API redeploy
- runner reconciliation survives runner restart
- package reupload restores missing archives
- Cloud UI and CLI expose matching deployment state
- screenshots and drill logs are saved under `.artifacts/agent-apps-launch-hardening/`
- no orphan containers, routes, temp dirs, or provider processes remain after validation
