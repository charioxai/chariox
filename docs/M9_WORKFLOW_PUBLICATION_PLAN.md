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

Implementation status:

- The gateway can still load a publication file directly for standalone drills.
- The kernel now owns workflow publication records inside session state.
- Local API and kernel-client request helpers support publication create, list,
  get, and disable.
- Shell command routing exposes publication records through
  `workflow publication ...`, so the CLI shell pane and `arroba-shell` use the
  same executor path.
- The gateway can load a kernel-owned publication by
  `ARROBA_PUBLICATION_SESSION_ID` plus `ARROBA_PUBLICATION_ID` when explicit
  workflow/endpoint env vars are not provided.

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

- HTTP and HTTPS/TLS
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

Pairing is optional per publication. A workflow publication can use
`auth.mode = "anonymous"`, bearer/API-key auth, registered Arroba principals, or
Arroba auth with `paired_senders.enabled = true`. If paired senders are not
enabled for that publication, the gateway does not expose the pairing endpoint
for that workflow.

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
workflow publication pair-code <publication> [--expires-ms N] [--max-uses N]
workflow publication redeem-code <publication> <pair-code> [display-name]
workflow publication senders <publication>
workflow publication revoke-sender <publication> <sender_id>
```

HTTP shape:

```text
POST /.well-known/arroba/publication/pair
```

The endpoint redeems the pairing code and returns the sender id plus the issued
credential once. The gateway must not log the raw credential.

Implementation status:

- Kernel session state now owns publication pairing codes and trusted sender
  records. Pairing codes store only a hash of the opaque code; trusted senders
  store only a credential hash.
- Local API, kernel-client helpers, and shell commands support creating pairing
  codes, redeeming codes, listing senders, revoking senders, and authenticating
  sender credentials.
- The gateway supports optional paired-sender auth per publication through
  `auth.mode = "arroba"` plus `paired_senders.enabled = true`.
- The gateway exposes `POST /.well-known/arroba/publication/pair` only when
  pairing is enabled for that publication, authenticates subsequent HTTP calls
  through the configured sender credential header, and forwards the sender
  identity in invocation metadata.
- Unit coverage verifies optional pairing behavior, redemption, sender auth,
  and revocation. The live publication drill now covers anonymous publication
  plus paired publication reject/redeem/invoke/revoke/reject.

## M9.6 HTTP Connector

Support `GET` and `POST` over HTTP or HTTPS/TLS. HTTPS is not a separate
connector; it is TLS configuration on the HTTP publication gateway.

Pipeline:

```text
HTTP request
-> auth
-> parser
-> input validation
-> kernel workflow invoke
-> response forwarding
```

TLS configuration:

- file/config: `tls.enabled`, `tls.key_file`, `tls.cert_file`
- env override: `ARROBA_PUBLICATION_TLS_KEY_FILE`,
  `ARROBA_PUBLICATION_TLS_CERT_FILE`, `ARROBA_PUBLICATION_TLS_ENABLED`
- if TLS is enabled without both key and cert files, gateway startup fails
  clearly rather than serving insecurely by accident

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

For long-running workflows, clients can connect, submit a request envelope,
receive accepted/status/final events, and keep the client interaction open
while the workflow runs. This is the primary fit for semantic
rendering/checkouts where generation should stream progress.

WSS is not a separate connector. It is WebSocket over the same gateway TLS
configuration used for HTTPS.

V1 endpoint:

```text
ws://host/.well-known/arroba/publication/ws
wss://host/.well-known/arroba/publication/ws
```

V1 client message:

```json
{
  "type": "invoke",
  "input": {}
}
```

V1 gateway messages:

```json
{ "type": "ready", "publication_id": "pub_..." }
{ "type": "accepted", "workflow_run": {} }
{ "type": "accepted", "queued": true, "result": {} }
{ "type": "status", "workflow_run": {} }
{ "type": "final", "workflow_run": {} }
{ "type": "error", "error": "..." }
```

Implementation status:

- The gateway exposes WebSocket upgrade handling at
  `/.well-known/arroba/publication/ws`.
- WebSocket auth reuses the publication auth config and HTTP upgrade headers.
- WebSocket invocation validates the configured input schema and invokes the
  same kernel workflow endpoint as HTTP.
- For direct workflow-run responses, the gateway can stream status/final
  messages by polling the kernel run state. Queued launches return an accepted
  queued event in v1.
- WSS works by starting the gateway with TLS, using the same config/env shape as
  HTTPS.
- Unit coverage verifies WebSocket invocation and validation errors. The
  publication live drill covers WS and WSS against the kernel-backed gateway.

## M9.8 IPC Connector

Local program/script integration without the interactive CLI:

```bash
arroba-workflow-call --config ./publication.config.json --input '{"task":"ship"}'
arroba-workflow-call --session-id <session> --publication-id <publication> --input-file input.json
```

V1 behavior:

- loads an exported `publication.config.json`, or looks up a kernel-owned
  publication by session id plus publication id
- accepts JSON input from `--input`, `--input-file`, or stdin
- validates the publication input schema before invoking the workflow endpoint
- invokes the same kernel workflow endpoint as HTTP and WebSocket
- returns the workflow invocation result as JSON on stdout
- treats local filesystem/kernel access as the v1 trust boundary and marks
  caller metadata as `auth=ipc`

Implementation status:

- Added `arroba-workflow-call` as a server package executable.
- Exported publication README files document local IPC invocation.
- Unit coverage verifies IPC-shaped caller metadata and validation failures.
- The publication live drill now invokes the exported publication package
  through `arroba-workflow-call`.

## M9.9 Export Command

CLI/shell commands:

```text
workflow publication create [workflow_ref] <endpoint_ref> [alias] [--route <route>] [--method POST] [--auth-json <json>] [--parser-json <json>] [--transport-json <json>] [--input-schema-json <json>] [--mode async]
workflow publication list
workflow publication show <publication>
workflow publication export <publication> <directory> [--kernel-url <url>]
workflow publication disable <publication>
```

Gateway export/package output will include publication config, launcher, README,
env var template, and example curl/websocket commands.

Implementation status:

- `workflow publication export <publication> <directory>` writes a deployable
  gateway package with `publication.config.json`, `.env.example`, `run.sh`, and
  `README.md`.
- The exported config is a file-based gateway config, so it can run without
  re-querying publication metadata at process startup as long as the target
  Arroba kernel remains reachable.
- The package preserves publication auth/parser/method/mode config and includes
  paired-sender pairing instructions when the publication enables paired sender
  auth.
- The publication live drill now exports a kernel-owned publication and starts
  the gateway from the exported `publication.config.json` before continuing to
  paired-sender coverage.
- Exported `.env.example` files include optional HTTPS/TLS variables for
  deployments that terminate TLS inside the Arroba gateway rather than at a
  proxy/load balancer.

## M9.10 Workflow-To-Workflow Drill

M9 does not add a dedicated workflow-to-workflow protocol. Instead, add a live drill where workflow A in one kernel calls workflow B in another kernel through B's published HTTP endpoint.

This proves the application shape while keeping protocol discovery and mesh semantics out of v1.

Implementation status:

- Added `pnpm --filter @arroba/cli run workflow-to-workflow-publication:drill`.
- The drill starts two isolated kernels and two workflow gateways.
- Workflow B is published over HTTP from the worker kernel.
- Workflow A is published over HTTP from the home kernel with a custom parser
  that calls workflow B's published HTTP endpoint, captures B's accepted
  workflow run id, and passes that metadata into workflow A's normalized input.
- The drill validates both A and B return accepted async workflow run metadata,
  proving cross-kernel workflow interaction through the v1 publication HTTP
  surface without adding the future mesh/discovery protocol.

## M9.11 Live Drills

Required drills:

- publish an existing workflow endpoint over HTTP/HTTPS: `pnpm --filter @arroba/cli run publication:drill`
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
- IPC invocation works through `arroba-workflow-call`
- workflow A calls workflow B through B's published HTTP endpoint in a separate kernel
- Docker-backed external client drill kicks off workflow runs over HTTP, HTTPS,
  WS, WSS, Slack, Discord, Telegram, WhatsApp, and Signal

Docker connector drill:

```bash
pnpm --filter @arroba/cli run publication:docker-connectors-drill
```

This drill proves provider-shaped ingress from outside the Arroba process. The
container client signs or authenticates requests using the same webhook
contracts as the real providers, then verifies each connector receives accepted
workflow run metadata. It does not replace a public-reachability drill through a
real deployed URL or tunnel.

Semantic URL renderer drill:

```bash
pnpm --filter @arroba/cli run semantic-url-renderer:drill
```

This drill validates the example application discussed during M9: a normal
static site exposes pages such as `/about` and `/contact`, while a wrapper route
like `/about/<prompt>` starts an async published workflow, returns a loading
page immediately, polls the workflow run, and eventually serves the workflow's
rendered HTML output. The v1 implementation keeps the publication gateway as
the workflow ingress and puts loading/polling behavior in the application layer.

## M9.12 Slack Connector

Slack is implemented as a connector-specific ingress path on the publication
gateway, feeding the same Arroba auth model as HTTP.

V1 behavior:

- verifies Slack request signatures with `x-slack-request-timestamp`,
  `x-slack-signature`, and the configured signing secret
- rejects stale or invalid signatures
- handles signed `url_verification` challenges directly without invoking the
  workflow
- accepts signed JSON events and slash-command form payloads
- normalizes Slack identity as `team_id:user_id` and maps it to an Arroba
  principal through `auth.external_identities`

Implementation status:

- Unit coverage verifies signed challenge handling does not invoke workflows,
  invalid signatures reject, and signed slash-command form payloads map to the
  configured Arroba principal.
- The publication live drill now creates a Slack-shaped publication, verifies
  signed URL verification, and invokes the workflow through a signed
  slash-command payload.

## M9.13 Telegram Connector

Telegram is implemented as a connector-specific ingress path on the publication
gateway, feeding the same Arroba auth model as HTTP and Slack.

V1 behavior:

- verifies `x-telegram-bot-api-secret-token` when `webhook_secret_env` is
  configured
- rejects missing or invalid webhook secrets
- extracts sender identity from `message.from`, `callback_query.from`, or
  `edited_message.from`
- normalizes Telegram identity as the Telegram user id string and maps it to an
  Arroba principal through `auth.external_identities`
- forwards the Telegram webhook envelope through the existing `webhook` parser

Implementation status:

- Unit coverage verifies webhook-secret rejection, accepted sender mapping,
  username metadata, and chat id metadata.
- The publication live drill now creates a Telegram-shaped publication,
  verifies invalid webhook-secret rejection, and invokes the workflow through an
  accepted Telegram webhook payload.

## M9.14 Discord Connector

Discord is implemented as a connector-specific ingress path on the publication
gateway, feeding the same Arroba auth model as HTTP, Slack, and Telegram.

V1 behavior:

- verifies `x-signature-ed25519` and `x-signature-timestamp` with the
  configured Discord public key
- signs/verifies the Discord-required `timestamp + raw_body` payload
- handles signed PING interactions (`type: 1`) directly without invoking the
  workflow
- rejects invalid signatures
- extracts sender identity from `member.user.id` or `user.id`
- normalizes Discord identity as `guild_id:user_id` when a guild is present,
  otherwise as the user id string, and maps it to an Arroba principal through
  `auth.external_identities`
- forwards the Discord interaction envelope through the existing `webhook`
  parser

Implementation status:

- Unit coverage verifies signed PING handling without workflow invocation,
  invalid signature rejection, accepted interaction sender mapping, username
  metadata, and guild/user metadata.
- The publication live drill now creates a Discord-shaped publication with a
  generated Ed25519 key pair, verifies PING handling, invalid signature
  rejection, and accepted signed interaction invocation.

## M9.15 WhatsApp Connector

WhatsApp is implemented as a connector-specific ingress path on the publication
gateway, feeding the same Arroba auth model as the other publication
connectors.

V1 behavior:

- handles Meta webhook verification over `GET` with `hub.mode=subscribe`,
  `hub.verify_token`, and `hub.challenge`
- verifies `x-hub-signature-256` HMAC-SHA256 over the raw request body when
  `app_secret_env` is configured
- rejects invalid verification tokens and invalid HMAC signatures
- extracts sender identity from `messages[0].from` or `contacts[0].wa_id`
- normalizes WhatsApp identity as the sender phone/wa id string and maps it to
  an Arroba principal through `auth.external_identities`
- forwards the WhatsApp webhook envelope through the existing `webhook` parser

Implementation status:

- Unit coverage verifies Meta webhook challenge handling, invalid verify-token
  rejection, invalid HMAC rejection, accepted sender mapping, and phone-number
  metadata.
- The publication live drill now creates a WhatsApp-shaped publication,
  verifies the challenge endpoint, invalid signature rejection, and accepted
  signed message invocation.

## M9.16 Signal Connector

Signal is implemented as a bridge-style webhook connector because Signal does
not provide a universal first-party bot webhook equivalent to Slack, Telegram,
Discord, or WhatsApp. The gateway verifies a bridge-supplied shared secret and
normalizes the bridge envelope into Arroba identity.

V1 behavior:

- verifies `x-signal-webhook-secret` when `webhook_secret_env` is configured
- rejects missing or invalid bridge secrets
- extracts sender identity from `envelope.sourceUuid`,
  `envelope.sourceNumber`, `envelope.source`, or matching top-level fields
- normalizes Signal identity as the bridge source UUID/number string and maps it
  to an Arroba principal through `auth.external_identities`
- forwards the Signal bridge webhook envelope through the existing `webhook`
  parser

Implementation status:

- Unit coverage verifies bridge-secret rejection, accepted sender mapping,
  source UUID metadata, and source number metadata.
- The publication live drill now creates a Signal-shaped publication, verifies
  invalid bridge-secret rejection, and invokes the workflow through an accepted
  Signal bridge webhook payload.

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
