# M9 Workflow Publication Plan

M9 turns selected workflow endpoints into externally callable services without moving workflow execution out of the Arroba kernel.

The published process is a publication gateway: it owns transport and client interaction, not output semantics.

It owns:

- listening on HTTP, WebSocket, IPC, and later other connectors
- authentication and caller identity
- request parsing
- input validation when configured
- invoking the kernel workflow endpoint
- sync, async, and streaming interaction control
- forwarding workflow-produced output to the client

It does not own:

- rendering HTML
- compiling or testing generated code
- validating domain-specific correctness
- transforming workflow output semantically
- deciding what the final product means

Agents and workflows own final output generation and domain validation. The gateway forwards the workflow's transport-shaped response mechanically.

## M9.1 Publication Model

Add workflow publication records that point to existing workflow endpoints.

Canonical fields:

```text
publication_id
session_id
workflow_id
endpoint_id
alias/name
enabled
transport config
auth policy
parser config
input schema reference
response mode
created/updated metadata
```

First implementation can load a publication file directly in the gateway. Kernel-owned publication CRUD comes after the gateway contract proves stable.

## M9.2 Endpoint Input Contracts

Parsers are endpoint-level concepts, not only publication-level concepts. An endpoint can define how raw connector input becomes normalized workflow input so CLI, shell, gateway, and future workflow-to-workflow calls can share input semantics.

V1 parser kinds:

- `json`
- `form`
- `query_params`
- `headers`
- `path_template`
- `regex`
- `webhook`
- `custom_command`

`custom_command` runs an external command with a JSON request envelope on stdin and expects normalized JSON on stdout. Future parser SDKs can wrap this contract.

## M9.3 Kernel Invocation Contract

The gateway submits normalized input to the kernel through existing workflow endpoint invocation first. The stable publication invocation envelope is:

```json
{
  "publication_id": "pub_...",
  "endpoint_id": "endpoint_...",
  "request_id": "req_...",
  "caller": {},
  "input": {},
  "mode": "sync"
}
```

The current kernel invocation prompt is a compatibility bridge. Later slices can add a native structured workflow invocation payload.

## M9.4 Publication Gateway

Add a deployable app at the same level as CLI and shell. The gateway loads a publication config, connects to an Arroba kernel, exposes configured transports, authenticates callers, parses input, invokes the workflow endpoint, and forwards the workflow output.

V1 transport:

- HTTP
- Slack connector
- Discord connector
- Telegram connector
- WhatsApp connector
- Signal connector

Later in the milestone:

- WebSocket
- IPC

## M9.5 Auth V1

Support production-usable simple auth:

- bearer token
- API key header
- paired sender
- registered Arroba user/team identity
- explicit anonymous mode

Every accepted request is associated with a caller identity in the normalized invocation metadata.

Arroba has one publication authorization model. Connectors are identity-proof
mechanisms that feed that model, not separate user systems.

- Connector ingress verification proves an external identity claim. Examples:
  Slack signing secret plus Slack workspace/user id, Telegram webhook secret
  plus Telegram user id, Discord interaction signature plus Discord user id,
  WhatsApp/Signal transport identity, or HTTPS/API-key checks for generic HTTP.
- Arroba identity authorization maps that verified external identity to one
  Arroba principal and decides whether that principal can invoke the
  publication.
- Publication policy can restrict which connectors a principal may use. A user
  linked to Slack can still be denied through Discord if the publication only
  allows that user through Slack.

Users should not register separately per connector. They register once in
Arroba and link external connector identities to that Arroba user/team. Paired
sender flow is a bootstrap/linking path for external callers that do not yet
have an Arroba identity or are intentionally limited to a publication. Anonymous
access remains an explicit publication policy for public HTTP-style services.

### Paired Senders

Paired sender auth is the workflow-publication equivalent of pairing a trusted
external caller with a published endpoint. A pairing code is only a bootstrap
credential; it is not used for steady-state request auth.

Flow:

1. The owner generates a short-lived pairing code for a publication or endpoint
   scope.
2. The sender redeems the code against the publication gateway.
3. The gateway asks the kernel to create a trusted sender record.
4. The sender receives a durable credential for subsequent requests.
5. Future requests authenticate as that sender and are recorded in invocation
   metadata.

Trusted sender records are kernel-owned state and include:

```text
sender_id
publication_id / endpoint scope
display name / external subject
auth method
credential hash or public key
allowed transports
created / last_used / expires_at metadata
revoked flag
```

V1 should support bearer-style issued tokens because they are easy to use from
curl, scripts, and other workflows. The record stores only a credential hash.
The design must keep room for signed requests later: a sender can redeem a
pairing code with a public key, then sign requests with timestamp and nonce
headers.

CLI/shell shape:

```text
/workflow publication pair-code <publication> [--endpoint <endpoint>] [--expires 10m] [--max-uses 1]
/workflow publication senders <publication>
/workflow publication revoke-sender <publication> <sender_id>
```

HTTP shape:

```text
POST /.well-known/arroba/publication/pair
```

The endpoint redeems the pairing code and returns the sender id plus the issued
credential once. The gateway must not log the raw credential.

## M9.6 HTTP Connector

Support `GET` and `POST`.

Pipeline:

```text
HTTP request
-> auth
-> parser
-> input validation
-> kernel workflow invoke
-> response forwarding
```

Response modes:

- `sync`: wait up to configured timeout
- `async`: return accepted run/status metadata immediately
- `stream`: SSE/WebSocket in later slices

HTTP passthrough output contract:

```json
{
  "kind": "http_response",
  "status": 200,
  "headers": {
    "content-type": "text/html"
  },
  "body": null,
  "body_artifact_id": "art_123"
}
```

If the workflow does not produce a transport-shaped response yet, the gateway returns workflow run metadata and final output message when available.

## M9.7 WebSocket Connector

For long-running workflows, clients can connect, submit a request envelope, receive progress events, and receive final output. This is the primary fit for semantic rendering/checkouts where generation should stream progress.

## M9.8 IPC Connector

Local program/script integration without the interactive CLI:

```bash
arroba-workflow-call publication-name --input input.json
```

## M9.9 Export Command

CLI/shell commands:

```text
/workflow publish <workflow_ref> <endpoint_ref>
/workflow publications
/workflow publication get <publication>
/workflow publication disable <publication>
/workflow export <publication> --out ./published-app
```

Export output includes publication config, launcher, README, env var template, and example curl/websocket commands.

## M9.10 Workflow-To-Workflow Drill

M9 does not add a dedicated workflow-to-workflow protocol. Instead, add a live drill where workflow A in one kernel calls workflow B in another kernel through B's published HTTP endpoint.

This proves the application shape while keeping protocol discovery and mesh semantics out of v1.

## M9.11 Live Drills

Required drills:

- publish an existing workflow endpoint over HTTP
- auth accepted/rejected
- paired sender code generation, redemption, accepted request, revoked request
- connector ingress verification plus Arroba identity authorization for Slack,
  Discord, Telegram, WhatsApp, and Signal
- JSON parser success/failure
- regex/path parser success/failure
- custom parser success/failure
- sync response returns workflow output or run metadata
- async response returns run id/status metadata
- artifact-backed HTTP response passthrough
- WebSocket stream returns progress/final output
- IPC invocation works
- workflow A calls workflow B through B's published HTTP endpoint in a separate kernel

## V2

- Dedicated workflow-to-workflow invocation protocol with signed caller identity, reply routing, status/result URLs, and optional streaming.
- Workflow mesh discovery, trust policies, capability metadata, quotas, revocation, and federation.
- Parser SDKs/libraries for Python, TypeScript, Rust, and other languages so users can define custom parsers programmatically while targeting the same stdin/stdout parser protocol.
- Packaged publication templates for common hosting targets.
- Connector plugin SDK for additional chat and collaboration surfaces such as Matrix, Mattermost, Google Chat, LINE, IRC, Nostr, Microsoft Teams, Feishu/Lark, Twitch, QQ, Zalo, Nextcloud Talk, Synology Chat, BlueBubbles/iMessage, Tlon, and self-hosted/custom channels.
- Connector-specific security contracts: provider webhook signature verification, raw-body HMAC checks where required, stable sender-id allowlists, mention/command gating for group contexts, per-connector rate limits, SecretRef-style credential indirection, and security-audit checks for dangerous public ingress.

## OpenClaw Reference Notes

OpenClaw source inspection was done from `https://github.com/openclaw/openclaw`
at commit `f1df354`. Relevant local files:

- `/tmp/openclaw-source/docs/channels/index.md`
- `/tmp/openclaw-source/docs/channels/pairing.md`
- `/tmp/openclaw-source/docs/cli/security.md`
- `/tmp/openclaw-source/docs/plugins/sdk-channel-plugins.md`
- `/tmp/openclaw-source/extensions/*/channel-plugin-api.ts`

Useful mechanisms to adapt:

- Keep connector transport code pluggable, but normalize inbound requests into
  one core invocation envelope.
- Make pairing and allowlists core concepts instead of each connector inventing
  unrelated trusted-sender storage.
- Split direct-message/private sender policy from group/channel policy.
- Prefer stable provider ids over mutable names, usernames, tags, or emails.
- Treat group/public-channel ingress as mention-gated by default.
- Keep provider webhook verification beside the connector because each provider
  has different signature, token, and raw-body requirements.
- Add a security audit command for publication/connectors that flags anonymous
  public ingress, wildcard sender rules, weak tokens, missing webhook
  signatures, missing TLS/proxy assumptions, and dangerous connector settings.
