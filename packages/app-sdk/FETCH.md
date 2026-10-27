# Worker Fetch contract (SDK 0.7)

The trusted bootstrap installs `chariox.http.fetch` as `globalThis.fetch` before
importing an App. Native `Request`, `Response`, `Headers`, `FormData`, `Blob` and
Web Streams remain available. The adapter sends only the existing five HTTP
stream operations over the worker SDK channel. It never invokes native network
Fetch, selects provider credentials, or creates a second permission path.

Anonymous HTTPS destinations and methods must be declared by the signed App.
Every redirect opens another stream through the same current-catalog and actual
connected-address checks. The kernel continues to reject raw authorization,
cookies, missing opaque connection authority and protected effect routes.
This adapter does not complete the remaining App connection/operation-receipt
authority work. App startup and precommit health checks retain the existing
kernel restrictions on effects.

## Supported transport behavior

The adapter returns native Response values, including streaming bodies, normal
body consumption methods, cloning, immutable response headers, final URL and
redirect status. FormData and URLSearchParams use native serialization. Uploads
are split into at most 64 KiB broker writes and retain backpressure; decoded
responses use bounded streaming transforms. Aborting before or after headers,
canceling the response body, or losing an operation reply cancels the exact
kernel handle without retrying body bytes. Early completed responses cancel an
unfinished upload without turning their received body into an upload error.

Redirect modes `follow`, `error`, and Node-style `manual` are supported, with a
maximum 20 hops under the original one-hour lifetime. POST 301/302 and 303 method
rewrites remove body headers. 307/308 reuse a retained immutable BodyInit source;
arbitrary ReadableStreams and prebuilt Requests with bodies cannot be replayed.
The public Request API does not expose its internal replay source, and the
adapter does not inspect private Undici fields or build an unbounded tee buffer.
Each replay is a new authorized network request, never an automatic retry after
unknown completion. A caller content-length is checked against streamed bytes
and omitted from the broker request so the kernel always owns HTTP framing.

Gzip, deflate and Brotli decode incrementally; up to three encoding stages are
accepted. Each decoded stage is capped at 256 MiB in addition to the kernel's
256 MiB encoded-body cap. Headers describing the encoded representation remain
visible as in native Fetch. The kernel's four streams per installation, 32 per
kernel, 64 MiB upload and 120-second network inactivity bounds remain in force.
Malformed or unsupported encodings fail and cancel the stream.

This is a Node App transport subset, not a browser origin/CORS implementation
or a claim to pass the entire Web Platform Tests suite. There is no ambient
cookie jar, HTTP cache, proxy, client TLS certificate, credential inclusion,
subresource-integrity handling, WebSocket or keepalive beyond the worker.
Only HTTPS URLs are accepted. Prebuilt Request body replay, raw-deflate servers
mislabeling content encoding, and original HTTP reason phrases are excluded.
Native Response statusText is empty because the broker does not expose a reason
phrase. `http.request` remains the separate 512 KiB buffered convenience API.

## Validation scope

`node --test --test-concurrency=1 packages/app-sdk/test/*.test.js` covers the SDK
peer and adapter, including multipart round trips, redirects and replay errors,
streaming compression, bounded read-ahead, upload pressure, abort, early-response
cleanup, content-length failures and native Response cloning. The shared
`test/fetch-contract.json` records actual response observations for the versioned
contract; raw HTTP calls keep their existing protocol 294 floor.

`node scripts/test-app-fetch-clients.mjs` installs the exact locked
`@octokit/request` 10.0.16 and `eventsource-parser` 4.1.0 into disposable outside-repo
scratch with npm scripts disabled. It checks JSON/query transport, redirects,
compressed and streamed downloads, binary uploads and fragmented UTF-8 SSE with
the parser's explicit buffer bound. The script removes the scratch afterward.

The bootstrap test uses a real inherited FD3 channel and a trap replacing
ambient Fetch. It proves that App imports observe the broker adapter and that
the complete request/gzip response/cleanup exchange crosses that channel. These
ordinary Node fixtures do not establish sandbox containment, live service
authentication, real kernel TLS delivery or a complete end-to-end release gate.
Hosted execution with the exact embedded Node 24.20 bundle and enrolled worker
still needs to exercise this SDK 0.7 layer together with the kernel HTTP broker.

Primary contracts:

- [Fetch redirect algorithm](https://fetch.spec.whatwg.org/#http-redirect-fetch)
- [Node 24.20 Web Streams and decompression](https://github.com/nodejs/node/blob/v24.20.0/doc/api/webstreams.md)
- [Octokit 10.0.16 custom Fetch implementation](https://github.com/octokit/request.js/blob/v10.0.16/src/fetch-wrapper.ts)
- [SSE parser 4.1.0 stream contract](https://github.com/rexxars/eventsource-parser/blob/v4.1.0/src/stream.ts)
