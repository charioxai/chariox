# Browser session import components

These components include an installable, attended Chrome MV3 source connector.
The connector reuses the consent flow, current-profile source reader and direct
encrypted relay transport in this directory. Final destination delivery uses the
runtime's private `browser_import_final_delivery` v1 capability and fails closed
when that capability or its exact authoritative result is unavailable.

## Chrome MV3 connector

Build an unpacked extension into a disposable directory (never a real Chrome
profile or the repository):

```sh
connector_output="$(mktemp -d)"
TYPESCRIPT_MODULE=/absolute/path/to/typescript/lib/typescript.js \
  node apps/browser-session-import/chrome-extension/package-extension.mjs "$connector_output"
```

In `chrome://extensions`, enable Developer mode, choose **Load unpacked**, and
select `$connector_output`. Remove that disposable directory after unloading the
extension. A release system may zip the directory contents without changing the
tree; it must apply normal extension signing/publication controls separately.
The source Chrome executable and profile are never modified by the packager.

The manifest has only `activeTab` initially. `cookies` and broad HTTP/HTTPS host
patterns are declarations under `optional_permissions` and
`optional_host_permissions`, not grants. When the user clicks the extension
action, its service worker captures that one source tab and opens a durable
extension page. Incognito and non-HTTP(S) source tabs are rejected. The page
reports only the current profile, source hostname and connector public-key
metadata. It neither probes nor lists cookie names or values.

Pairing is attended, independently anchored and one-use:

1. The extension creates a non-extractable P-256 sender key in memory and emits
   a metadata-only bootstrap request. The user copies it into Chariox's already
   authenticated Browser Import panel and selects one attached session/kernel
   and destination there.
2. Chariox returns a short-lived authenticated bootstrap containing a random
   `bootstrap_id`, the selected relay URL, daemon, kernel public key, immutable
   destination selection, compatible protocol/capability version and echoed connector
   sender/source metadata. The extension validates and freezes this bootstrap
   before creating the pairing challenge. It is the independent trust anchor;
   the later pairing response cannot select or replace those fields.
3. The extension creates a separate 128-bit enrollment nonce. The user returns
   that challenge to the same authenticated Chariox panel, which supplies a
   single-use relay token in a pairing response that echoes the `bootstrap_id`,
   nonce and every pinned field. The extension rejects any mismatch, including
   a different well-formed P-256 kernel key, relay, daemon or destination. Both
   pasted inputs are cleared from the DOM immediately.
4. A direct connector-to-relay socket must match the independently pinned daemon
   and kernel key; a handshake cannot replace the bootstrap key. The same sender
   key is retained for every consent and source-read request.
5. The extension shows the exact destination and domains. Only the final
   **Allow hosts and import** click calls `confirmAndDeliver()`. Its first synchronous
   action is `chrome.permissions.request` for `cookies` plus only
   `*://<confirmed-host>/*` origins. No consent round trip or cookie read precedes
   that gesture.
6. Closing/cancelling aborts owned waits, sends the bounded existing kernel
   cancellation, closes the relay, ignores late replies and reports a fixed
   result code. The extension does not retry one-use operations.

Before the confirmation gesture, the connector snapshots `cookies` and each
exact host permission independently and reserves those needs with the shared MV3
service worker. Chrome's permission-added event plus post-grant activation marks
only grants absent from that snapshot as connector-owned. Completion, denial,
cancellation, timeout, page disconnect and delivery failure all release the
lease. A grant is removed only when connector-owned and no other active connector
page needs it; permissions that predated the operation are never removed.

The connector declares no storage permission and uses no extension storage,
content script, externally connectable endpoint, web-accessible resource or page
message bridge. Cookie values exist only in the short-lived source batch passed
to the private delivery adapter. They never enter extension storage, DOM text,
logs, browser history, analytics, prompts, screenshots, pairing data, progress,
results, consent requests or other public protocol requests.

### Final-delivery adapter contract

`chrome-extension/delivery-adapter.mjs` accepts only the paired relay's `deliver`
method. The relay owns the private encrypted envelope defined by
`browser_import_final_delivery` v1; the UI adapter deliberately does not duplicate
that schema. It binds delivery to the prepared request ID and exact frozen
selection, accepts only one ordered authoritative result per confirmed domain,
and maps `no_cookies` to the public `sign_in_required` status. Aggregate, missing,
extra, reordered or malformed results fail with the fixed
`browser_import_delivery_unavailable` code. The metadata-only `request` method
never accepts cookie values.

### Runtime and Web integration

The packaged connector now implements the runtime half. Remaining Web work is:

1. In chariox-cloud branch `codex/browser-import-product-ui`, replace
   `configuredBrowserImportProductAdapter = null` only with an adapter using the
   Web client's existing direct kernel/relay transport. `detect()` must expose
   metadata only. `start(selection)` creates one request ID, first returns the
   authenticated immutable bootstrap for the selected kernel, and only then
   returns the nonce-bound pairing response for that same bootstrap. It returns
   one idempotently cancellable handle. Do not send runtime payloads through a
   Cloud HTTP handler.
2. Forward progress with that exact request ID and selected-domain count. Accept
   completion only from the authoritative kernel result; the extension and Web
   UI must never infer success from permission grant, source read or socket send.
3. Run the connector regressions here, the compatible runtime command tests and
   the Web adapter/coordinator/entry tests together before enabling the capability.

## Standard managed-browser boundary

The managed Chromium profile is the product's durable destination. Chromium,
not Chariox, owns saved-password import and storage. Managed startup does not
write Chromium's `Preferences` file, so a user can use the browser's attended
password import UI and Chromium retains the user's browser-owned settings.
Chariox does not read, parse, log or convert that payload. This is separate from
the cookie connector described below and does not make password import available
to an unattended agent.

The exact fixed headed launch switches are `--password-store=basic`,
`--no-first-run`, `--no-default-browser-check`, `--disable-sync`,
`--disable-dev-shm-usage`, `--disable-gpu`,
`--remote-debugging-address=127.0.0.1`, and
`--remote-debugging-port=9222`. The launch also supplies the persistent
`--user-data-dir`. It adds `--restore-last-session` only when that directory has
a restorable session, and adds `--new-window -- <url>` only for URL recovery;
otherwise the configured start URL follows `--`. Desktop startup and URL
recovery use this same launcher.

The launch never uses `--unsafely-treat-insecure-origin-as-secure`,
`--no-sandbox`, or another sandbox-disabling switch. Local development pages
must use a genuinely secure context when they need secure-context browser APIs;
the managed browser does not promote an HTTP origin. Loopback CDP preserves the
browser controller/action path without changing page security semantics.

Chrome Sync and Chromium's automatic import from a host default browser are
intentionally not transfer paths. A slice cannot see an arbitrary host Chrome
profile unless the user explicitly makes source data available.
`--password-store=basic` keeps the browser store inside the slice profile
instead of depending on a host keyring; it does not add OS-keyring encryption,
so slice/home access controls are the store's protection boundary.

Browser-native password import transfers saved passwords only. It does not
transfer an authenticated web session, passkeys, payment credentials, Chrome
Sync state, host keychain entries, device registration or device-bound keys.
Cookie import transfers only the explicitly approved cookie fields documented
below; it does not transfer local storage, IndexedDB, Cache Storage or service
worker state. Those origin stores persist after they are created inside the
managed profile, but they are not imported from Chrome.

### Service limits

| Service class | Expected result |
| --- | --- |
| Google accounts and Workspace | Cookie import is best effort. [Device-bound session credentials](https://developer.chrome.com/docs/web-platform/device-bound-session-credentials) and [passkeys](https://support.google.com/accounts/answer/13548313) do not move; Google may require a fresh sign-in or verification. Chrome Sync is disabled. |
| Microsoft Entra / Microsoft 365 | Cookie-only SSO may be incomplete. [Conditional Access session controls](https://learn.microsoft.com/en-us/entra/identity/conditional-access/concept-conditional-access-session), sign-in frequency, PRT-backed cookies and token protection may require a registered device or fresh authentication. |
| GitHub.com | A supported cookie session may transfer, but [GitHub documents](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/about-authentication-to-github) separate `github.com` and `gist.github.com` cookies. New-device checks, SAML SSO, 2FA and sudo-mode reauthentication still apply. [Passkeys](https://docs.github.com/en/authentication/authenticating-with-a-passkey/about-passkeys) remain with their authenticator. |
| Other sites | Treat fixture success as no portability promise. Expiry, revocation, origin storage, MFA, risk checks and device binding may require normal sign-in. |

Never weaken or bypass a service check. Report `Sign in required` rather than
claiming that an incomplete or rejected session imported successfully.

## Kernel recovery execution barrier

The kernel checks durable pending-import metadata by Room before authorizing
import consent, admitting an Environment Action or dispatching a browser/controller
command. This also blocks
when the current Environment has not been restored, when its identity differs
from the recorded import, or when the durable read fails. Marking recovery verified
does not reopen execution; acknowledged journal cleanup must remove the row first.
Queued human and agent dispatch waiters recheck this barrier. Active cancellable
executors request cancellation through the existing controller path. Controller
release and execution cancellation remain available while execution is blocked.

This is an execution check, not an atomic import coordinator. The destination
still must acquire exclusive ownership, drain or fence previously dispatched
work, stop independent browser writers, and create the durable record before
changing cookies. Passive display streams are not fenced by this check. The
worker does not independently own the home kernel's recovery record. No public
destination command, profile/key provisioning or end-to-end import is enabled by
this barrier.

The internal controller bridge requires a trusted recovery journal and forwards
it into the cookie transaction. It refuses a missing journal, blocks existing
recovery, waits for journal preparation before the first cookie write, and retains
the encrypted record after successful browser readback. This connects the real
controller adapter to journal storage; it does not supply the kernel's durable
acknowledgement, consent transport or exclusive browser-writer ownership.

## Trusted confirmation flow

`prepareChromeCookieImport` composes the internal reader with kernel consent and
Chrome permission requests. Preparation snapshots and freezes the selected
metadata and asks the kernel to prepare consent, without reading cookies.
The packaged extension UI calls `confirmAndDeliver()` directly in its confirmation
click handler; lower-level trusted callers may use `confirmAndRead()`. Both call
Chrome's permission API before yielding the user gesture,
requests only the selected hosts and cookie permission, then approves and claims
the matching kernel request before reading. Confirmation is single-use.

Preparation expires within two minutes, including permission and request waits.
Caller abort, denial, expiry and failed reads stop outstanding work and send at
most one bounded best-effort kernel cancellation. `cancel()` reports whether that
cancellation was acknowledged. It does not close the shared relay or retract a
Chrome API operation already sent. Late results cannot authorize more reads.
The returned cookie batch contains secrets and belongs only to the trusted caller;
the flow retains no batch in its state. `confirmAndDeliver()` keeps the request
alive only through private relay delivery, scrubs the batch and releases its
permission lease on every outcome. Successful lower-level reads leave the source
claim for the destination operation; their caller must cancel abandoned delivery.

The packaged extension manifest declares the optional permissions its UI
requests. Its background coordinator removes only operation-acquired grants once
no active lease needs them, including grants reported by a late permission-added
event. Preexisting grants are never removed. The attended connector displays
pairing and source/destination selection; it exposes no page-message endpoint and
never imports a profile automatically.

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
The request callback receives a second argument containing the read's owned
`AbortSignal`. Transports must honor it. Finishing, cancelling or timing out the
read aborts its outstanding requests without closing a shared relay connection
or aborting the caller's signal. This discards local waits; it does not retract
a claim already received by the kernel or send a new consent-cancellation request.

This module imports the shared TypeScript request builders. Both the disposable
browser fixture and connector packager transpile that source when assembling an
MV3 extension. This reader module itself creates no transport or page-message endpoint.

`readApprovedChromeCookies({chrome, scope, sourceTabId, authorize, signal,
timeoutMs})` snapshots the selected source tab, store and domains. It rejects
incognito tabs and ambiguous or mismatched stores. Existing cookie and exact-host
permissions are required; the extension page requests optional permission from
the explicit confirmation gesture before calling it.

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

The internal `applyControllerCookieImport` bridge requires a trusted encrypted
journal dependency before resolving a browser target. It forwards that journal
to the transaction and retains pending state after readback. Its disposable
Chrome test verifies retained recovery data and rejection when the journal is
missing. This does not provide kernel key provisioning or durable completion.

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
node --test apps/browser-session-import/chrome-import-consent-flow.test.mjs
node --test apps/browser-session-import/cookie-import-flow.test.mjs
```

Run the connector regressions with an already installed TypeScript compiler (the
packager test creates and removes only a disposable directory):

```sh
TYPESCRIPT_MODULE=/absolute/path/to/typescript/lib/typescript.js \
  node --test apps/browser-session-import/chrome-extension/*.test.mjs
```

The connector tests cover manifest permissions, direct user-gesture ordering,
exact-host narrowing, preexisting and concurrently shared grant cleanup,
current-profile metadata discovery, independently bootstrapped enrollment
sender/key/nonce binding, valid-key substitution, replay and wrong-kernel
rejection, authoritative per-domain result reconstruction, fixed-error redaction,
encrypted packaged delivery and package layout. Consent/source/relay suites cover
mid-operation cancellation, nonce replay and late-result suppression.

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
It also exercises the confirmation flow from a real extension-page click using
Chrome's permission API. The fixture grants permissions in its manifest so no
native authorization dialog is automated. Denial, cancellation, expiry and
permission ordering are covered separately by unit tests. Kernel replies remain
fixtures, not live consent or pairing. The HTTP fixture route excludes extension
URLs so it cannot replace the extension's HTML or modules with mock responses.

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
This transport creates no relay token issuer, page endpoint or cookie-application
route. The packaged connector supplies the attended enrollment UI around it.

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
checkpoints. Protocol 321 and `browser_import_final_delivery` v1 wire the paired
connector's encrypted delivery to the private destination claim. No public
cookie-apply request exists. The remaining product dependency is the Web adapter
that issues the authenticated bootstrap/pairing response and presents progress.

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
kernel lifecycle state. Without the optional journal, snapshots remain in memory.
Product recovery after process death or browser restart remains unhandled. This must not
be advertised as durable atomic import.

### Internal recovery storage

`openCookieImportJournal({directory,key,binding})` provides encrypted pending-record
storage for the future Environment executor. `applyCookieImport` accepts it as an
optional `journal` dependency: pending records block another import, the encrypted
snapshot and intended cookies are synced before mutation, and uncertain failures
retain the record. Verified application retains it for durable kernel completion;
verified rollback clears it. Revocation
during preparation clears it without touching the browser. This is internal
transaction integration only, not kernel lifecycle, connector or transport wiring.
It does not authorize import or establish a second credential vault.

Successful browser readback is not a durable browser commit. The product executor
still needs durable outcome/quarantine state and recovery replay before this
adapter can safely serve real imports across browser or machine crashes.

`recoverCookieImport({journal,store,runExclusive,authorize})` is an internal replay
operation. The kernel must stop the previous executor and all cookie writers,
provide a verified fresh store, and authorize recovery under exclusive ownership.
Replay validates the record, removes attempted cookie identities, restores only
their previous values and verifies the complete snapshot. Unrelated loss remains
an error, not permission to rewrite another domain. Missing records, denial and
verification failures keep recovery required. Replay accepts valid expired records
but does not restore expired cookies. Verification discounts natural expiry from
the saved snapshot; session cookies retain their original session semantics.

Verified replay returns a receipt but retains the record. The kernel must durably
record recovery before discarding it and lifting quarantine. This function does
not implement that lifecycle, acquire exclusion itself, or prove browser disk
durability. Its tests use synthetic stores, not managed-browser acceptance.

The kernel must provide a private, local-filesystem Environment directory under
its own state root, with trusted ancestors, owned by the current Unix user and
inaccessible to group/other users. The final directory and record cannot be
symlinks. The kernel must retain the 32-byte encryption key through its existing
protected key lifecycle so a replacement executor can recover the record.
The module neither generates nor persists a key. It copies the caller's key and
clears that copy on `close()`; callers retain ownership of their input buffers.
This adapter supports macOS and Linux, not Windows or network filesystems.

The interface has four operations:

- `prepare(bytes)` encrypts one nonempty Buffer of at most 8 MiB with AES-256-GCM,
  a random nonce, and authenticated `userId`, `roomId`, and `environmentId`.
  It creates a mode-0600 pending file exclusively and syncs the file and directory
  before returning a receipt. An existing record is never overwritten, including
  an incomplete write. Browser mutation must not begin before this succeeds.
- `read()` returns `null` only for an absent record, or authenticated
  `{bytes,receipt}`. The caller must clear the returned plaintext Buffer after
  use. Corrupt, oversized, non-private, multiply linked, or incorrectly bound
  records require recovery instead of returning cookie bytes.
- `discard(receipt)` removes only a matching authenticated record and syncs the
  directory. The caller may invoke it only after verified browser recovery or a
  durably recorded successful outcome. A receipt is a record identifier, not
  authorization. Missing, invalid, or stale receipts do not delete a record.
- `close()` clears the owned key and closes the directory handle. It is
  idempotent, but rejects while an operation on that handle is pending.

Same-process operations on the same directory inode reject overlap. Exclusive
file creation also prevents competing processes from overwriting a pending
record. **This is not a cross-process executor lock.** The kernel must serialize
the complete snapshot/apply/recover/discard transaction across processes and
worker threads and stop the previous executor before transferring ownership.
It must also prevent pages, service workers, network responses, and other actors
from changing cookies while recovery is being verified. A private directory
does not defend against a malicious process with the same operating-system UID.

Filesystem I/O failures disable the affected handle. Do not treat reopening an
empty journal as permission to resume an Environment after an uncertain cleanup.
The kernel must retain durable quarantine/outcome state independently and verify
the browser before lifting it. Errors expose fixed codes and a
`recoveryRequired` flag, not file paths, cookies, or underlying OS errors.
Explicit buffers are cleared where ownership permits; JavaScript and native
crypto internals cannot promise complete process-memory erasure.

`cookie-import-journal.test.mjs` exercises real files, wrong keys/bindings,
tampering, size/permission/link checks, concurrent handles and writer processes,
receipt protection, failure disabling, abrupt writer death, and an interrupted
write under an OS file-size limit. Fixtures contain no real credentials and are
removed afterward. These are process-crash storage tests, not a power-loss,
browser rollback, Web/TUI, or managed-machine acceptance result. Key provisioning,
recovery payload validation, executor serialization, durable quarantine and
product transaction integration remain required before real sign-in import.

Run `node --test apps/browser-session-import/cookie-import-transaction.test.mjs`
for the destination tests. With `PLAYWRIGHT_MODULE` set, run the matching
`cookie-import-transaction.browser-test.mjs` for disposable Chrome validation of
sign-in, cancellation rollback and an untouched partitioned control cookie.
`cookie-import-cdp.test.mjs` covers seven deadline, context and uncertain-transport cases.

`applyControllerCookieImport` is an internal bridge to the existing
`BrowserCdpClient`. It uses the controller's registered target session and checks
browser generation and live document identity before application and at each
authorization checkpoint. It snapshots source records and scope before awaiting
consent. The trusted consent callback receives frozen target/scope metadata, not
cookie values. Missing consent/exclusion/completion functions, denied consent and
stale identities fail closed. The caller's exclusive operation covers target
selection and the complete transaction.

The controller's cookie-writer fence auto-pauses newly created page and worker
targets, stops service workers, disables page scripts, forces current page
sessions offline, stops loading, waits for tracked requests to settle, and
freezes each page. Verified import or recovery must record its durable outcome
and remove the encrypted journal before the controller releases the fence. An
uncertain result retains the in-process fence. Restart recovery acquires a fresh
fence before reading or restoring the journal. The real browser fixture runs a
continuous response-driven cookie writer and proves it cannot race import or
durable cleanup, then proves the page resumes after release.

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

This ledger is not an authentication mechanism by itself. Its callers supply
identities from the authenticated relay and scope from trusted kernel state,
never from a cookie payload or a sender's approval boolean. Protocol 321 uses the
same live client/key/attachment binding for delivery, claims the destination
under exclusive Environment ownership and creates durable quarantine before the
private controller command. Active cancellation is routed by request ID, and the
authoritative result contains exact metadata-only selected-domain coverage.

The bridge is called only by the private protocol-321 delivery path. Run its
`controller-cookie-import.browser-test.mjs` with `PLAYWRIGHT_MODULE` for a
disposable, sandboxed Chrome test with the real controller connection.

Enable the guarded Web progress/results adapter only for that exact runtime
capability. Remaining acceptance work includes managed process/browser-crash
drills.
Complete the security and service validation matrix in
`docs/BROWSER_SESSION_IMPORT_RESEARCH.md` before importing real sign-ins.

Contracts come from the [Chrome cookies API](https://developer.chrome.com/docs/extensions/reference/api/cookies),
[CDP CookieParam](https://chromedevtools.github.io/devtools-protocol/tot/Network/#type-CookieParam)
and [CDP cookie storage](https://chromedevtools.github.io/devtools-protocol/tot/Storage/#method-getCookies).
No third-party implementation code was copied.
