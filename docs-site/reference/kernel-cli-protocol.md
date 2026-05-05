# Kernel-CLI Protocol

This page describes the current local protocol between the Arroba Kernel and the TypeScript CLI.

It reflects the current implementation, not the long-term remote/federated design.

Current shared local daemon protocol version: `2`.

Primary implementation sources:

- `apps/kernel/src/runtime_transport.rs`
- `apps/cli/src/ipc.ts`
- `apps/cli/src/ipc-requests.ts`

## Transport

The primary local transport is a WebSocket connection from the CLI to the kernel.

Default endpoint:

- `ws://127.0.0.1:${ARROBA_KERNEL_PORT}/kernel`

The CLI still retains compatibility fallback logic for the older local socket path, but the main path is WebSocket and event-driven.

## Frame Types

The protocol uses JSON frames.

Incoming frames from CLI to kernel:

- `request`
- `subscribe`
- `unsubscribe`

Outgoing frames from kernel to CLI:

- `response`
- `event`

## Request/Response Frames

### Request

```json
{
  "type": "request",
  "request_id": "uuid-or-random-id",
  "request": { "...": "LocalDaemonRequest payload" }
}
```

### Response

```json
{
  "type": "response",
  "request_id": "same-id",
  "response": { "...": "response payload or null" },
  "error": null
}
```

When an error is present:

```json
{
  "type": "response",
  "request_id": "same-id",
  "response": null,
  "error": {
    "code": "transport_or_request_code",
    "message": "human-readable message",
    "retryable": true
  }
}
```

## Subscription Lifecycle

The CLI opens the WebSocket and then subscribes to one session/attachment event stream.

### Subscribe

```json
{
  "type": "subscribe",
  "request_id": "id",
  "session_id": "session-id",
  "attachment_id": "attachment-id",
  "resume_from_event_id": 123
}
```

`resume_from_event_id` is optional and is used for reconnect/resume.

### Unsubscribe

```json
{
  "type": "unsubscribe",
  "request_id": "id"
}
```

## Event Frames

All pushed events are wrapped like this:

```json
{
  "type": "event",
  "event_id": 124,
  "event": {
    "event": "event_name",
    "...": "event payload"
  }
}
```

Current event kinds:

### `terminal_output`

Provider output and other terminal stream records.

Payload:

- `records`

### `runtime_notices`

Kernel/runtime notices for the current attachment.

Payload:

- `notices`

### `assistant_message_completed`

Structured completion signal for the final assistant message from OpenCode.

Payload:

- `session_id`
- `provider_run_id`
- `agent_id`
- `message_id`
- `completed_at_ms`

Clients use this as transcript metadata for assistant-message grouping. Runtime status must continue to come from `session_snapshot.agent_activity` or the session prompt state fallback.

### `session_snapshot`

Current session snapshot plus current provider run snapshot.

Payload:

- `session`
- `provider_run`
- `agent_activity`

The session snapshot is also how the CLI hydrates workflow definitions and other current workspace state on attach/rejoin.

Provider run token usage uses distinct fields for cumulative and context-window counts:

- `usage.total_tokens`: cumulative provider-run token usage
- `usage.last_tokens`: latest provider-reported turn usage
- `usage.context_tokens`: current model context occupancy when the provider reports a value that does not exceed the known context window
- `usage.context_window`: model context limit

### `session_unavailable`

Sent when the subscribed session is no longer available.

Payload:

- `session_id`
- `message`

### `heartbeat`

Liveness heartbeat for an active subscription.

Payload:

- `session_id`

### `transport_resumed`

Emitted after reconnect/resubscribe when the stream resumes.

Payload:

- `session_id`
- `resumed_from_event_id`

### `transport_closed`

This is emitted client-side by the CLI transport wrapper when the WebSocket closes.

Payload:

- `message`

## Recovery Semantics

The current transport supports:

- durable `event_id`
- reconnect/resubscribe
- `resume_from_event_id`
- bounded recent-event replay
- heartbeat/liveness

The kernel keeps a bounded recent-event history per session for replay on reconnect.

## Backpressure

The kernel enforces a bounded outgoing queue per connection.

If a client becomes too slow, the kernel closes the socket with policy close code `1008` and reason:

```text
kernel transport overloaded; reconnecting
```

The CLI can then reconnect and resubscribe with `resume_from_event_id`.

## Scope

This protocol page covers the current kernel-CLI local transport only.

It does not describe:

- future agent transports
- relay transport
- directory/discovery
- workflow run scheduling protocol
