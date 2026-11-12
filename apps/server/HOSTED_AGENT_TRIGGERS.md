# Agent HTTP trigger ingress

MP-08 / MP-09 / MP-10 / MP-11: the runtime ingress process receives public HTTPS HTTP trigger requests, asks an admission adapter who may fire the trigger and which kernel it belongs to, then sends a single encrypted invocation to the kernel through the opaque relay. It never replays a request after a lost acceptance. Conversation history stays on the browser-to-kernel encrypted connection.

Build with `pnpm --filter @chariox/server run build`, then start `node apps/server/dist/hosted-agent-trigger-main.js` with `CHARIOX_AGENT_TRIGGER_PUBLIC_ORIGIN=https://your-ingress-origin`, `HOST=127.0.0.1` and `PORT=43110`. The public origin must be an exact HTTPS origin. The edge must route `/agent-triggers` and `/agent-triggers/*` to this service, preserving the original Host and forwarding HTTPS protocol. `/ready` probes the process. No database or provider account is needed here.

Choose exactly one admission adapter (setting both, or neither, stops the process at start):

## Self-hosted (when no admission URL is set)

`CHARIOX_AGENT_TRIGGER_DEPLOYMENTS_FILE=/path/deployments.json`, a file only its owner can read (mode 600):

```json
{
  "relay": {"url": "wss://relay.example", "token": "<operator relay token>"},
  "deployments": [
    {"id": "nightly", "kernelId": "<kernel id>", "sessionId": "<session id>", "triggerId": "<HTTP trigger id>", "accessToken": "<32 or more random characters>"}
  ]
}
```

The kernel admits only keys it paired, so pair the ingress with its kernel once. On the kernel create a terminal pairing link (`CreateTerminalPairingLink`, terminal type `cli`), then run `CHARIOX_AGENT_TRIGGER_DEPLOYMENTS_FILE=/path/deployments.json node apps/server/dist/hosted-agent-trigger-main.js pair <link>`. This joins the link through `relay` and saves the ingress relay key as `agent-trigger-identity.json` next to the deployments file (mode 600, never overwritten). Without it the ingress refuses to start. The paired key has the standing of a paired terminal; the ingress itself sends nothing but HTTP trigger invocations.

Rule: a request to `/agent-triggers/<id>/` is admitted when it carries `Authorization: Bearer <accessToken>` of that deployment. A wrong or missing token is 401, an unlisted id is 404. The operator relay credential is used for the ingress-to-kernel connection only and never reaches the caller. `relay.url` must be `wss://`; `ws://` is accepted on loopback only.

## Admission service

`CHARIOX_AGENT_TRIGGER_ADMISSION_URL=https://admission.example/deployments/{id}/admission` (`{id}` is the deployment id; HTTPS, or HTTP on loopback). The ingress sends `POST` with `?access=read|operate&method=<HTTP method>`, a JSON body that is empty or `{"publicKeyThumbprint": "<hex>"}`, and these request headers when present: `cookie,x-chariox-browser-session,csrf-token,x-csrf-token,origin,sec-fetch-site,sec-fetch-mode,accept` (override the list with `CHARIOX_AGENT_TRIGGER_ADMISSION_HEADERS`, comma separated). It also sends `Host` and `X-Forwarded-Host` of the public origin and `X-Forwarded-Proto: https`. Reply `2xx` with

```json
{"deployment": {"id": "<id>", "kernelId": "...", "sessionId": "...", "triggerId": "..."},
 "bootstrap": {"relayUrl": "wss://...", "relayToken": "...", "relayTokenExpiresAt": "<ISO time>", "target": {"daemonId": "<kernel id>", "kernelRef": "<kernel id>"}}}
```

`bootstrap` is required when the request carried a `publicKeyThumbprint`. Any other status is a refusal; only 400, 401, 403, 404, 429 and 503 pass through, everything else becomes 503. The service's `Set-Cookie` headers and its `Content-Security-Policy`, `Content-Security-Policy-Report-Only`, `X-Content-Type-Options`, `X-Frame-Options`, `Referrer-Policy`, `Cross-Origin-Opener-Policy` and `Strict-Transport-Security` headers are relayed to the caller. `CHARIOX_AGENT_TRIGGER_SIGN_IN_PATH=/path` sends a signed-out browser navigation (401 on an HTML `GET`) to `<path>?returnTo=<address>`; without it a refusal is a plain 401.

Admission receives authentication and deployment metadata only, never original request bodies or query values. The kernel enforces the original 65536-byte serialized application-payload budget independently from the preserved HTTP byte envelope (at most 65536 decoded bytes). The HTTP body limit remains 65536 bytes. A 30000-character JSON value or 30000-byte plain-text body continues to fit. Invalid encoding or either exceeded budget returns `payload_too_large`.
