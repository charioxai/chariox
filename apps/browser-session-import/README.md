# Browser session import conversion

This is the first import implementation step, not a usable importer. Nothing
here reads a user's browser profile, requests extension permissions, contacts a
kernel, or installs cookies in a product Environment.

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
