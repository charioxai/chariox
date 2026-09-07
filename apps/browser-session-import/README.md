# Browser session import conversion

This is the first import implementation step, not a usable importer. Nothing
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
MV3 extension. Use the same Playwright environment variable; optionally set
`CHARIOX_TEST_CHROMIUM` to an already installed Chromium/Chrome for Testing binary
when its revision differs from Playwright's default. It never downloads a browser.
The test extension has permission for the fixture host only. It checks actual
HttpOnly and partitioned reads, selected-domain output and source preservation.
The generated extension and browser profile are removed in `finally`. These are
fixture-only permissions and an injected authorization callback, not product
consent, pairing, or a distributable connector.

## Required before product integration

Implement the MV3 connector and source-profile/site selection, kernel-owned
consent and destination authorization, encrypted transport, bounded decoding,
expiry and replay protection, cancellation and transactional application with
rollback. Add shared protocol versioning and Web/TUI progress and results when
that transport is implemented. No shared protocol shape changes in this step.
Complete the security and service validation matrix in
`docs/BROWSER_SESSION_IMPORT_RESEARCH.md` before importing real sign-ins.

Contracts come from the [Chrome cookies API](https://developer.chrome.com/docs/extensions/reference/api/cookies),
[CDP CookieParam](https://chromedevtools.github.io/devtools-protocol/tot/Network/#type-CookieParam)
and [CDP cookie storage](https://chromedevtools.github.io/devtools-protocol/tot/Storage/#method-getCookies).
No third-party implementation code was copied.
