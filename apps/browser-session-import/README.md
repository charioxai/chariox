# Browser session import components

These are internal import components, not a usable importer. Cookie transfer is
not wired to a user's profile or product Environment. The source
reader uses Chrome's extension APIs when called by a trusted connector. It does
not request permissions, transmit cookies or register a web-accessible endpoint.

## Source reader

`readKernelApprovedChromeCookies({chrome, requestId, selection, sourceTabId,
request, signal, timeoutMs})` is the connector entry point. `request` must use the
initiating client's authenticated kernel transport. The entry point snapshots
the selected metadata with the shared kernel-client builders, claims approval
once, and requires the matching `source_authorized` response before Chrome access,
between queries and before returning. A boolean, wrong request ID or wrong phase
cannot authorize a read. Timeout/cancellation also prevents a late claim response
from issuing further requests. Failed reads leave no destination writer; an
unacknowledged source claim expires under the kernel's original consent lifetime.

This module imports the shared TypeScript request builders. The disposable browser
fixture transpiles that source when assembling its MV3 extension; a distributable
connector still needs its normal bundling, pairing and permission UI. No actual
transport or page-message endpoint is created by this module.

`readApprovedChromeCookies({chrome, scope, sourceTabId, authorize, signal,
timeoutMs})` snapshots the selected source tab, store and domains. It rejects
incognito tabs and ambiguous or mismatched stores. Existing cookie and exact-host
permissions are required; the eventual popup must request optional permission
from an explicit user gesture before calling it.

The trusted `authorize(selection)` callback receives an immutable selection and
must verify live kernel pairing, request expiry, consent and destination identity.
It is called before cookie queries, after each query and before returning. A
callback returning `true` is a test seam, not an implemented authorization system.
Never expose this adapter to arbitrary page messages or accept a wire-supplied
approval boolean. The eventual kernel must independently authorize application.

The reader uses one normal cookie store in the profile running the extension. It
cannot read arbitrary Chrome profiles on disk. Queries select one approved domain
and partition site at a time. Chrome's domain filter may also return subdomains;
the reader discards those unless separately queried under explicit approval.
Partition queries preserve both cross-site-ancestor variants. No cookie write or
delete API is used.

The reader returns validated Chrome API records, not CDP CookieParams. The
destination independently validates those source records against its trusted
scope and converts them where they are applied. Returning CDP parameters from
the source would bypass that format contract and fail destination validation.

Cancellation and a maximum 30-second total deadline settle even if Chrome has not
settled an API call. Chrome does not offer cancellation of the underlying cookie
read; late results are discarded, not published. Timers and abort listeners are
removed. Each API result and the accumulated batch are capped at 512 records;
conversion enforces the payload limit before another query. Chrome's API has no
pagination, so the first returned array is allocated by Chrome before this cap
can be checked. API errors become fixed codes and contain no original payload.

`prepareChromeCookieBatch(source, scope)` converts plain Chrome cookies API
records to CDP CookieParam records. It validates the entire batch before returning
anything and does not mutate the input. It performs no I/O. The caller must own
the authorized scope; a sender-supplied scope is not authorization.

The returned `cookies` contain secrets. Never log them, expose them to agents,
serialize them into ordinary action history, or write them to evidence. Errors
contain fixed codes only. `summary` contains counts and domain names, which still
must stay within the authorized user's import UI. JavaScript strings are not
zeroized by this module.

## Supported conversion

- Exact approved cookie domains and one exact source store. Approval does not
  automatically cover subdomains. Domain cookies retain domain scope, while
  host-only cookies use a URL without a domain attribute.
- Secure, HttpOnly, path, SameSite and future persistent expiry. Session cookies
  omit expiry; unspecified SameSite remains unspecified.
- Partitioned cookies with both an explicitly approved top-level site and a
  boolean cross-site ancestor field. Missing or unknown semantics fail closed.
- At most 512 cookies, 32 domains, 32 partition sites and 512 KiB of serialized
  input records. Cookie name and value together are limited to 4096 UTF-8 bytes;
  paths are limited to 1024 characters. Duplicate identities, expired records,
  invalid prefixes and unsupported fields reject the whole batch.

The hostname contract currently accepts ASCII/punycode DNS-style names and IPv4,
not IPv6 literals. This is not a public-suffix validator. Chrome's cookies API
does not expose every CDP cookie property, so this is not full-fidelity profile
migration. It does not transfer passwords, origin storage, passkeys or
device-bound private keys.

## Validation

Run dependency-free tests with:

```sh
node --test apps/browser-session-import/chrome-cookie-batch.test.mjs
node --test apps/browser-session-import/chrome-cookie-reader.test.mjs
node --test apps/browser-session-import/kernel-source-reader.test.mjs
node --test apps/browser-session-import/cookie-import-flow.test.mjs
```

For the browser test, set `PLAYWRIGHT_MODULE` to an existing Playwright ESM module
and run `node --test apps/browser-session-import/chrome-cookie-batch.browser-test.mjs`.
It uses installed Chrome, opens one disposable headless browser and closes it in
`finally`. It does not download software or use a real profile. Fixture page
requests are intercepted, and no real service login is attempted.

The test creates separate source and destination contexts. It verifies a
signed-out destination authenticates after conversion, source cookies and an
unrelated destination cookie remain unchanged, HttpOnly hides the cookie from
page scripts, host-only scope excludes subdomains, and partition metadata
survives CDP installation. It also checks neither `--no-sandbox` nor the unsafe
insecure-origin flag is present. The source records are constructed from fixture
cookies, so this does not yet test an actual extension's cookies API.

The real-browser test also verifies same-name/path host-only and domain cookies
coexist in the destination. Their leading-dot distinction is part of Chromium's
cookie identity, so the converter must not reject that pair as a duplicate.

`chrome-cookie-reader.browser-test.mjs` runs the reader inside a real disposable
MV3 extension. Use the same Playwright environment variable and set
`TYPESCRIPT_MODULE` to an installed TypeScript module to compile the shared
browser crypto into the disposable extension. Optionally set
`CHARIOX_TEST_CHROMIUM` to an already installed Chromium/Chrome for Testing binary
when its revision differs from Playwright's default. It never downloads a browser.
The test extension has permission for the fixture host only. It checks actual
HttpOnly and partitioned reads, selected-domain output and source preservation.
The extension encrypts its output using the existing Chariox relay envelope.
The native relay crypto decrypts it before the destination operation applies
the cookies in a separate browser context. Reloading a controlled fixture page
then proves the destination authenticates without exposing HttpOnly cookies to
page scripts. Only ciphertext crosses the extension evaluation boundary.
The generated extension and browser profile are removed in `finally`. The fixture
now uses the grant-aware reader and shared request builders, with simulated
metadata-only kernel replies. It also verifies boolean approval is rejected.
Permissions and transport replies are fixtures, not live kernel consent, pairing
or a distributable connector.

`@chariox/kernel-client/browser-relay-crypto` exposes the existing Cloud WebCrypto
implementation using the shared relay envelope type. Its implementation comes
from `chariox-cloud/apps/web/src/kernel/relay-crypto.ts` at Cloud commit
`bb190be7bf62e10310b5c345d4a057f72167e96c`. Browser/native interoperability tests
cover both directions, tampering and wrong recipient keys. No encryption scheme
or wire shape changes here. Cloud still imports its original local copy; moving
that caller to this package requires a separate Cloud dependency change.

For protocol 316 import pairing, retain the keypair returned by
`createRelayKeypair()` and pass it as the third argument to `encryptRelayPayload`.
`relayPublicKeyThumbprint(publicKeyBase64)` matches the kernel's SHA-256 hex over
the encoded string, not the decoded P-256 bytes. Generated private keys are
non-extractable. Pass the kernel key established during trusted enrollment as
the third argument to `decryptRelayPayload`; never take that expected key from
the received envelope. Ordinary requests can retain the two-argument ephemeral
behavior. These APIs do not implement enrollment, persistence or transport.

Protocol 317 additionally binds import replies to the nonce of the exact encrypted
request attempt. `decryptBrowserImportResponse` in `relay-response.mjs` pins the
trusted kernel key, verifies the encrypted `request_nonce` and unwraps `response`.
The outer relay request ID is not authenticated and cannot replace this check.
Retries must use fresh encryption nonces. Missing bindings and old bare responses
fail closed. This decoder does not open a socket, pair a client or grant access.

`connectBrowserImportRelay` now provides the internal socket transport for the
metadata-only consent requests. It uses the existing `client_connect`,
`client_request` and `client_response` frames and shared encryption. Its caller
must supply an already authorized relay token, exact daemon ID, retained sender
keypair and kernel public key obtained through trusted pairing. A key received
in a handshake cannot establish trust. Production connections require `wss:`;
unencrypted `ws:` is restricted to loopback development. URL credentials,
query strings and fragments are rejected.

The connector snapshots the sender identity and whitelists bounded selection
metadata. It does not accept general kernel commands or cookie payloads. Replies
must match the expected consent phase and identifier as well as the pinned kernel
and encrypted request nonce. There are at most eight pending requests, bounded
frames and send backlog, and deadlines of five seconds by default with a maximum
of 30 seconds. Abort and close reject pending work and discard late results.
The transport does not reconnect or retry a one-use consent request automatically.
Errors contain a fixed message, never the original transport payload.

Run `relay-connector.test.mjs` with `WS_MODULE` pointing to an already installed
`ws` ESM module. The tests use disposable loopback servers and the real encryption,
decoder and source-reader modules. They cover identity mismatch, replay, metadata
filtering, mid-read revocation, deadlines, cancellation, disconnect, pending limits
and malformed frames. Backpressure and a stalled handshake use socket doubles.
The server and Chrome cookies API are fixtures, not a deployed relay/kernel or a
real browser profile. No credentials or cookies from a user's account are read.
This transport creates no enrollment UI, relay token issuer, extension endpoint
or cookie-application route, and is not yet installed in a product connector.

This fixture tests encrypted component composition, not delivery through a live
relay, authenticated kernel pairing or destination admission. Its recipient key,
consent callback and independent destination scope are controlled test inputs.

## Required before product integration

Protocol 313 now exposes kernel-owned prepare/approve/cancel consent requests.
Shared request builders whitelist metadata, excluding cookie payloads. The kernel
checks interactive attachment ownership, verified relay client identity and Room
membership. The consent ledger binds the source key and realm to the exact
selection and current destination generation/document. A changed binding requires
new consent, and agent/service callers cannot approve it. The router tests cover
local and verified relay callers, identity/scope changes, replay and navigation.
Protocol 314 adds a one-use source claim and repeated source authorization checks.
Both revalidate the initiating identity and current destination against the same
selection. Cancellation or expiry releases a reading reservation, since it owns
no destination writer. Destination execution has a separate private claim and
cannot start from an unclaimed approval.

The grant-aware reader wires these requests to the source authorization
checkpoints. Source-connector pairing, permission UX and live encrypted delivery
still need integration. No public cookie-apply request exists yet; destination
authorization and exclusion remain separate requirements.

`applyCookieImport` and `createCdpCookieStore` provide the internal destination
operation. They are not routed from the Browser Controller, a web endpoint or a
kernel request yet. The operation validates input, reads a bounded destination
snapshot, rejects existing-cookie conflicts unless overwrite was explicitly
approved, applies the batch, and verifies values and supported semantics. It
returns only counts and approved domain names. The CDP adapter binds reads and
writes to the selected target's browser context.

If cancellation or authorization loss follows a completed write, the operation
removes only the attempted identities, restores their previous values and checks
the snapshot. Same-name cookies in a different partition remain untouched.
Unrelated-cookie loss, including browser eviction, fails verification. It is not
repaired by silently rewriting unapproved domains. Unknown destination fields,
opaque partitions and ambiguous identities are rejected before mutation.
Snapshots are capped at 10,000 records and 4 MiB after the browser API returns.

The caller must supply `runExclusive(operation)` using the kernel's exclusive
Environment operation and `authorize()` bound to that exact user, destination,
generation, request and overwrite consent. It must stop page/network cookie
writers as well as competing Chariox actions while the snapshot is in use.
The injected functions in tests are not implemented kernel exclusion or consent.
`createCdpCookieStore` bounds discovery and each read/write/remove operation to
five seconds by default. `timeoutMs` may lower that limit but cannot disable or
increase it. All commands in one operation share its deadline, including an
entire deletion batch. Transport failure or timeout makes that store unusable;
it issues no more CDP commands even if the abandoned command later replies.
Errors are fixed codes, with no original browser error or cookie payload.

Cancellation waits for an in-flight write to settle or reach its bounded timeout.
CDP cannot retract a command already sent. If a mutation might have reached the
browser, a timeout reports `recoveryRequired: true` and does not attempt rollback
through that uncertain connection. The caller must keep the Environment
quarantined and recover using a verified fresh connection. It must close its
owned CDP sessions to discard pending transport requests. The adapter does not
close caller-owned sessions or claim that a late write was cancelled. Injected
non-CDP stores must provide equivalent bounded behavior.

Any transport/write exception sets `recoveryRequired: true`, even if the cleanup
read looks correct, because a lost acknowledgement can hide a late mutation.
Rollback failure and unrelated-cookie loss also set it. The kernel must quarantine
that Environment until recovery verifies it. Fixed error codes never include
original transport errors or cookies. These recovery flags are not yet wired to
kernel lifecycle state. Snapshots are in memory, not a crash-safe journal, so
process death or browser restart during import remains unhandled. This must not
be advertised as durable atomic import.

Run `node --test apps/browser-session-import/cookie-import-transaction.test.mjs`
for ten destination tests. With `PLAYWRIGHT_MODULE` set, run the matching
`cookie-import-transaction.browser-test.mjs` for disposable Chrome validation of
sign-in, cancellation rollback and an untouched partitioned control cookie.
`cookie-import-cdp.test.mjs` covers seven deadline, context and uncertain-transport cases.

`applyControllerCookieImport` is an internal bridge to the existing
`BrowserCdpClient`. It uses the controller's registered target session and checks
browser generation and live document identity before application and at each
authorization checkpoint. It snapshots source records and scope before awaiting
consent. The trusted consent callback receives frozen target/scope metadata, not
cookie values. Missing consent/exclusion functions, denied consent and stale
identities fail closed. The caller's exclusive operation covers target selection
and the complete transaction.

Persistent Chrome profiles can report a default context ID in target metadata
that `Storage.getCookies` does not accept. The CDP adapter queries
`Target.getBrowserContexts`: explicit contexts retain their ID; the confirmed
default uses an omitted ID. Unknown IDs reject instead of falling back. Older
Chrome versions that supply an ID but cannot identify their default are unsupported
for that case. A real-controller fixture covers persistent-profile import and
stale generation/document rejection; the existing fixtures cover explicit contexts.

The kernel now owns an internal `BrowserImportAdmission` ledger. It binds a
random one-use request to the initiating user, source attachment/store, exact
Room/Environment, generation, Tab/document revision, domain/partition selection,
and overwrite choice. Approval, claim and active authorization compare that
immutable binding. Requests expire after two minutes. Concurrent claims have
one winner; cancellation revokes authorization without freeing an active
Environment slot until the trusted executor reports verification or recovery.
The ledger holds at most 128 bounded metadata records and never stores cookies.
Unclaimed expired entries are reclaimed; active or cancelled writers remain
reserved until completion. A fresh kernel rejects old request IDs.

This ledger is not an authentication mechanism or a complete consent flow. Its
internal callers must supply identities from authenticated transports and scope
from trusted kernel state, never from a cookie payload or a sender's approval
boolean. Protocol 313 provides the human prepare/approve/cancel handlers described
above, but there is no source-connector pairing or cookie application authorized
by these methods in the product. A connector-bound claim path and live encrypted
IPC/relay acceptance still need implementation and validation. The ledger
alone does not quarantine a controller after process death, stop page/network
writers, validate DNS cookie semantics, or provide durable recovery. Those
remain requirements of the Environment executor.

The bridge is not yet called by a kernel request or installed in the slice image.
It does not consume kernel consent or implement writer quiescence or recovery.
No controller apply command is exposed. Run its
`controller-cookie-import.browser-test.mjs` with `PLAYWRIGHT_MODULE` for a
disposable, sandboxed Chrome test with the real controller connection.

Implement the MV3 connector and source-profile/site selection, consent UI and
destination application authorization, encrypted transport, bounded decoding,
expiry and replay protection, cancellation and transactional application with
rollback. Add shared protocol versioning and Web/TUI progress and results when
that transport is implemented.
The remaining destination work includes kernel integration, writer quiescence,
durable encrypted recovery state and process/browser-crash drills.
Complete the security and service validation matrix in
`docs/BROWSER_SESSION_IMPORT_RESEARCH.md` before importing real sign-ins.

Contracts come from the [Chrome cookies API](https://developer.chrome.com/docs/extensions/reference/api/cookies),
[CDP CookieParam](https://chromedevtools.github.io/devtools-protocol/tot/Network/#type-CookieParam)
and [CDP cookie storage](https://chromedevtools.github.io/devtools-protocol/tot/Storage/#method-getCookies).
No third-party implementation code was copied.
