# Web Bot Auth

MP-08 / MP-09 / MP-10 / MP-11: a project can identify its kernel's agent HTTP
traffic using Ed25519 HTTP Message Signatures. This implements RFC 9421 and
`draft-ietf-webbotauth-httpsig-protocol-01` (29 October 2026). Signing identifies
the operator; a receiving site still decides whether to accept its requests.
Cloudflare signed-agent registration is separate and is not enabled by this
feature.

The owner opts in exact project ids through the kernel's user configuration:

```toml
[web_bot_auth]
projects = ["project-id"]
```

The default list is empty. Both ordinary and managed kernels use the same policy.
The kernel config API also accepts `web_bot_auth.projects` as a JSON array and
`web_bot_auth.signature_agent` as an origin. Project controls in the terminal-os
UI are not part of this change.

An enrolled kernel requests a key using its existing Cloud kernel credential.
Cloud publishes the public JWK and returns the private seed once. The kernel
stores it in its Vault, outside the credential-handle catalog and tool results.
Keys expire within 24 hours and refresh when fewer than five minutes remain.
Re-enrollment replaces the cache binding, so a revoked grant's key is not reused.
Cloud can revoke a key independently of its expiry and excludes keys belonging
to inactive enrollments from its directory.

Self-hosted kernels publish their own directory. Configure its public origin:

```toml
[web_bot_auth]
projects = ["project-id"]
signature_agent = "https://operator.example"
```

Reverse-proxy only `/.well-known/http-message-signatures-directory` to the kernel's
runtime MCP HTTP listener. Keep the listener private; its MCP endpoints retain
their normal authentication. The public directory contains no private material
and cannot unlock the Vault or issue a key. The first opted-in request prepares
the Vault key before contacting the destination. During rotation, the directory
keeps the previous public key until that key expires. Empty project policy
removes the self-hosted keys from discovery. HTTP on loopback is supported for
local drills; public origins require HTTPS.

`chariox.http_request` and `chariox.http_request_with_credential` sign the final
URL after credential injection. Their signatures cover the method, complete
target URI, authority, and the matching `Signature-Agent` dictionary member.
When a body is supplied, its SHA-256 `Content-Digest` is covered too. Redirects
must be requested explicitly so each destination receives a fresh signature.

The kernel browser installs CDP Fetch interception before an agent opens or
navigates a tab. Navigations and HTTP subresources receive fresh signatures over
the method, target URI, authority and directory URL. Browser bodies are not
covered. Keys pass only through the trusted controller pipe; they are not browser
page data or tool results. Owner takeover retains the tab's operator identity.
Project opt-out clears that project's live browser signer without interrupting
other projects. Local App asset interception retains its own Fetch ownership.

The `Signature-Agent` member points directly to the well-known directory URL,
as required by draft-01. Key ids are RFC 7638 JWK SHA-256 thumbprints. Each
signature has a fresh nonce and a lifetime of at most five minutes, bounded by
the key's expiry. Signing failures return an error instead of silently sending
an unsigned opted-in request.

The independent verifier in
`scripts/e2e-stack/web-bot-auth-verifier.mjs` checks the wire signature, directory
selection, key validity, request scope, body digest when covered, and nonce reuse.
Its regression test includes unsigned requests, tampering, wrong credentials,
expiry, untrusted directories and removed keys. The companion Cloud drill
`apps/web/scripts/newui-web-bot-auth-browser-drill.mjs` drives the production
terminal-os shell over a local relay with a real provider and the kernel browser.
It uses the self-hosted directory and records source and binary identities outside
the repositories.

References: [RFC 9421](https://www.rfc-editor.org/rfc/rfc9421),
[Web Bot Auth draft-01](https://datatracker.ietf.org/doc/html/draft-ietf-webbotauth-httpsig-protocol-01).
