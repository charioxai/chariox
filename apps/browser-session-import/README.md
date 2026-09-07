# Browser session import components

These are internal import components, not a usable importer. Nothing
here is wired to a user's profile, kernel or product Environment. The source
reader uses Chrome's extension APIs when called by a trusted connector. It does
not request permissions, transmit cookies or register a web-accessible endpoint.

## Source reader

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
The generated extension and browser profile are removed in `finally`. These are
fixture-only permissions and an injected authorization callback, not product
consent, pairing, or a distributable connector.

`@chariox/kernel-client/browser-relay-crypto` exposes the existing Cloud WebCrypto
implementation using the shared relay envelope type. Its implementation comes
from `chariox-cloud/apps/web/src/kernel/relay-crypto.ts` at Cloud commit
`bb190be7bf62e10310b5c345d4a057f72167e96c`. Browser/native interoperability tests
cover both directions, tampering and wrong recipient keys. No encryption scheme
or wire shape changes here. Cloud still imports its original local copy; moving
that caller to this package requires a separate Cloud dependency change.

This fixture tests encrypted component composition, not delivery through a live
relay, authenticated kernel pairing or destination admission. Its recipient key,
consent callback and independent destination scope are controlled test inputs.

## Required before product integration

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
The store/CDP transport must enforce bounded command timeouts. Cancellation
waits for a pending write to settle before attempting cleanup; it must not release
the Environment while an acknowledged mutation is still pending.

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

Implement the MV3 connector and source-profile/site selection, kernel-owned
consent and destination authorization, encrypted transport, bounded decoding,
expiry and replay protection, cancellation and transactional application with
rollback. Add shared protocol versioning and Web/TUI progress and results when
that transport is implemented. No shared protocol shape changes in this step.
The remaining destination work includes kernel integration, writer quiescence,
durable encrypted recovery state and process/browser-crash drills.
Complete the security and service validation matrix in
`docs/BROWSER_SESSION_IMPORT_RESEARCH.md` before importing real sign-ins.

Contracts come from the [Chrome cookies API](https://developer.chrome.com/docs/extensions/reference/api/cookies),
[CDP CookieParam](https://chromedevtools.github.io/devtools-protocol/tot/Network/#type-CookieParam)
and [CDP cookie storage](https://chromedevtools.github.io/devtools-protocol/tot/Storage/#method-getCookies).
No third-party implementation code was copied.
