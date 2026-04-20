# M5 Relay Plan

## Goal

Add relay-backed remote transport without moving workspace, provider-run, or workflow authority out of the daemon/kernel.

The relay is:

- an independent app
- transport-only
- implemented in Rust
- optional for local-only use

The open-source implementation must support self-hosted relay usage without depending on any external managed identity/discovery service.

Mandatory privacy rule:

- all user-generated payloads that cross the relay path must be session-scoped end-to-end encrypted
- this includes:
  - prompts
  - workflow inputs and outputs
  - transferred or attached artifacts
- the relay must only handle opaque ciphertext plus the minimum routing/liveness metadata
- this rule applies equally to self-hosted and later managed relay deployments

## Core Decisions

- the relay is a separate Rust app, not part of the daemon or CLI process
- the same CLI supports both:
  - local direct daemon connection
  - relay-mediated remote daemon connection
- connection mode is explicit in v1:
  - `local`
  - `relay`
- the daemon opens an outbound connection to the relay
- daemon identity is stable and persisted:
  - `daemon_id`
  - `machine_id`
  - optional daemon alias
- one daemon uses one active relay connection at a time in v1
- multiple relay endpoints may be configurable later, but simultaneous multi-relay presence is out of scope for the first slice
- self-hosted relay mode initially uses static/shared credentials for bootstrap and drills; M5.5 replaces runtime reliance on shared credentials with relay realms, pairing, and scoped tokens
- self-hosted relay mode still requires end-to-end encrypted user payloads; trusted deployment ownership does not change the transport model
- any later managed identity/discovery service remains outside this repository and must integrate cleanly with the same transport boundaries

## Non-Goals

- remote workflow interconnection
- remote agent hosting across machines
- multi-relay active fanout for one daemon
- hosted account/auth/discovery service details beyond the issuer contract introduced by M5.5
- durable event storage in relay
- making relay the workspace or workflow authority

## Architecture

### Components

1. `arroba-kernel`
- runtime authority
- owns sessions, provider runs, workflows, prompt queues, and routing

2. `arroba-cli`
- one client app for both local and relay-backed use

3. `arroba-relay`
- websocket transport broker
- routes client traffic to connected daemons
- keeps liveness and minimal routing metadata

### Transport topology

Local:

`CLI <-> Daemon`

Remote:

`CLI <-> Relay <-> Daemon`

Authority rule:

`CLI -> Relay -> Daemon`

The daemon remains the only runtime authority in both paths.

## Identity Model

Required daemon-side identities:

- `daemon_id`
  - stable identifier used for routing through the relay
- `machine_id`
  - stable identifier for the hosting machine/user context
- `os_name`
  - plain liveness metadata reported by the daemon for relay display naming, for example `macOS` or `Linux`
- `kernel_started_at_ms`
  - first-start timestamp for the live kernel process; the relay uses this to assign deterministic live kernel display ordinals
- `daemon_alias`
  - optional human-friendly label for CLI targeting

Routing rule:

- relay routes by `daemon_id`
- CLI may target by alias when available
- alias resolution maps to `daemon_id`
- relay kernel lists expose relay-scoped live aliases like `machine 1 (macOS)` based on live kernel registration order and reported OS name; those aliases are addressable by relay metadata and kernel-to-kernel routing
- relay machine lists remain grouped by stable `machine_id`; when multiple kernels run on the same machine, `/machine kernels <machine-ref>` exposes each addressable live kernel alias
- user-owned approval/rename state remains in the home kernel; relay aliases are not durable user names and may be overridden in the UI by local renames

## Configuration Model

### Daemon

The daemon must be configurable with:

- relay enabled/disabled
- relay URL
- daemon identity
- machine identity
- daemon alias
- relay credential/token

### CLI

The CLI must be configurable with:

- connection mode
- local daemon URL when using local mode
- relay URL when using relay mode
- relay credential/token
- target daemon id or alias

For CLI-side relay configuration, prefer `ARROBA_RELAY_TOKEN` plus `/relay use <ws-url>` so the token does not have to be typed into visible terminal scrollback. `/relay use <ws-url> <token>` remains available for self-hosted/manual testing.

### v1 mode rule

The CLI should not silently decide between local and relay paths in the first slice.

Use explicit modes:

- `local`
- `relay`

An `auto` mode may be added later once the remote model is stable and debuggable.

## Self-Hosted Mode

The open-source path must support:

- user-run relay
- static/shared credentials
- explicit daemon selection by id or alias

This is the baseline remote story for the repository itself.

## Planned Managed Integration Boundary

Later, an external service may provide:

- managed identity
- discovery
- token issuance

That service is intentionally outside this repository.

The relay protocol and daemon/CLI transport model should not depend on that service existing.

M5.5 defines the compatibility boundary for that future service:

- relay realms
- scoped relay token claims
- self-hosted and hosted token issuers
- remote CLI pairing
- remote machine pairing
- caller identity propagation into the kernel

See `docs/M5_5_RELAY_IDENTITY_PLAN.md`.

## Protocol Shape

Reuse the daemon-owned request/event model as much as possible.

Required relay message classes:

- daemon register
- daemon heartbeat
- client request
- daemon response
- client subscribe
- daemon event
- unsubscribe
- connection close/error

Required runtime properties:

- request correlation ids
- pushed event ids preserved end to end
- reconnect/resubscribe behavior
- liveness/heartbeat
- relay-visible payloads limited to routing/liveness metadata and opaque encrypted user-content envelopes

## Delivery Order

### Slice 1. Daemon Identity

Status: done


Add persisted:

- `daemon_id`
- `machine_id`
- optional `daemon_alias`

Store under daemon runtime state so relay registration has a stable identity.

### Slice 2. Relay App Skeleton

Status: done


Create a new app:

- `apps/relay`

Provide:

- websocket accept loop
- daemon/client connection roles
- in-memory connection registry
- heartbeat/liveness handling

### Slice 3. Daemon Relay Connector

Status: done


Add daemon-side background connector:

- connect to relay
- register identity/capabilities
- send heartbeat
- reconnect on disconnect

### Slice 4. Client Relay Connector

Status: partial


Add CLI relay mode:

- connect to relay
- target daemon by id/alias
- send request through relay for the first narrow request surface
- receive encrypted relay-backed kernel event subscriptions

### Slice 5. Request/Response Proxying

Status: partial


Support a narrow first surface:

- list sessions
- get session state
- attach to session

This slice is now implemented end to end through relay.

Encrypted event subscription forwarding is also now implemented for the relay path.
The relay path now also supports the narrow interactive session request surface needed for remote terminal use:

- submit/cancel/complete prompt requests
- resize requests
- focus/cycle/list agents
- session config updates
- provider metadata/status/login requests

Keep durable reconnect/resume semantics and broader terminal transport hardening for the next slice.

Relay-backed kernel event subscriptions now support reconnect/resubscribe with `resume_from_event_id`
and replay of recent encrypted events from the daemon-side relay event cache.

### Slice 6. Remote Terminal Flow

Add:

- submit prompt
- terminal input
- resize
- output/event streaming

### Slice 7. Reconnect and Resume

Add:

- client reconnect through relay
- daemon reconnect to relay
- event resume via preserved event ids where supported by daemon

### Slice 8. Tests and Drills

Add:

- relay integration tests
- daemon register/disconnect/reconnect tests
- client remote attach tests
- remote terminal smoke drills

## Exit Criteria

- a daemon can register to a relay and stay connected
- a CLI can connect through relay to a chosen daemon by id or alias
- remote prompt submission and output streaming work
- relay failure/disconnect does not move runtime authority away from the daemon
- self-hosted relay usage works without any external managed service
