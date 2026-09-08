# App HTTP streaming

These modules implement the anonymous HTTPS subset of the SDK 0.6/protocol 294
worker contract through the existing BackendBroker and single WorkerPeer. They
do not create a second listener, select a credential, or execute a protected
critical operation.

`AppHttpPolicy` derives exact origins/methods and protected-service origins from
the same verified package as the retained `AppCatalog`. App-chosen connection or
operation IDs never create authority. Anonymous transport rejects those IDs and
every request to an origin containing a declared critical effect. Connection and
single-use critical receipt integration remain required Phase 1 work.

`HttpTransport` connects directly to a checked numeric address, verifies the
actual socket peer before sending TLS or HTTP bytes, and uses the original URL
host for TLS verification and HTTP authority. No environment proxy, automatic
redirect/retry, ambient cookie jar, native add-on, or provider credential
participates. The owning task polls its single Hyper HTTP/1 connection; there is
no detached connection driver or pool.

Initial limits are four streams per owner/installation and 32 per kernel, two
64 KiB chunks queued in each direction, a 64 MiB request, a 256 MiB response,
one-hour absolute lifetime, and 120 seconds without actual socket progress.
Headers are limited to 64 entries/16 KiB. DNS has ten seconds, at most 32 returned
addresses, and 16 tracked driver tasks. These are policy limits, not performance
evidence. The existing lifecycle service owns one shared `HttpLimits` pool.

Hickory 0.26.2 supports Rust 1.88. Its minimal Tokio/system-config features read
Linux resolv.conf and macOS's global SystemConfiguration DNS entry. This does not
reproduce NSS, hosts-file overrides, multicast DNS, or macOS per-domain scoped
resolution. Hostnames are absolute DNS names; there is no public resolver
fallback or App-selected configuration. Every returned address is checked,
including mixed public/private answers. The custom runtime handle retains DNS
drivers, aborts and joins ordinary cancellation, and retains the stream lease
until actual I/O future destruction. Per-exchange DNS cache storage is disabled.

`HttpStreams` owns opaque handles for one actual worker, retaining its private
preparation lease and exact verified catalog. Targets cannot transfer between
catalog objects. Pending starts reserve bounded capacity but open no socket.
Typed `HttpJob` messages use the existing durable writer's read-only IMMEDIATE
transaction to check current publisher/installation immediately before enqueue.
Headers/read/write repeat that same current-catalog fence. No DB commit follows
a socket effect. Shared eight-operation admission stays owned through actual
frame publication or discard, including queued cancellation and disconnected
callers.

Opening acknowledges only after the first task poll checks the original request
budget. Once acknowledged, the stream keeps its original one-hour lifetime;
subsequent pulls retain each IPC request's original absolute budget. Handles,
unread buffers, socket tasks and DNS drivers share the lifetime reservation and
actual worker lease. Draining signals handles synchronously and joins all tasks;
canceling that wait preserves join ownership for a later resume. EOF consults the
retained task result so a full response queue cannot hide transport failure.

SDK `http.open/write/headers/read/cancel` use the same worker peer. One read
(including headers) and one write can be in flight; uploads never interleave.
Headers are replayable, and empty response queues return `pending` after at most
one second. Body chunks are consumed once. Host-only publication guards retain
the exact stream, operation gate and admission permit until the peer writer
finishes the response frame. An unpublished open/read/write response cancels
that stream. Cancellation after a partial frame closes the peer; cancellation
after a completed frame cannot produce a second failure for that request.
Incoming work waits within its original budget if the preceding operation is
finishing publication; concurrent body operations remain rejected. Unknown wire
completion must never be retried. Cleanup needs no new admission permit.
Buffered `http.request` composes the methods under one original
30-second-or-shorter deadline with a 512 KiB request/response cap. It is not a
separate RPC. Encoded responses remain raw bytes. Fetch redirect/replay,
compression, multipart and SSE acceptance still require broader validation;
this implementation does not claim Fetch conformance. WebSocket is excluded.

The 23 Rust source fixtures cover policy/actual-address enforcement, DNS joining,
real private HTTP sockets for SSE/multipart/backpressure/limits, exact signed
catalog ownership, generation/revocation, pending/unread capacity, failed EOF,
interrupted uploads, and resumable cleanup. Two inherited-channel fixtures use a
fixed libc worker and fixed test Echo/Paused transports: a complete exchange,
and a full upload queue whose cleanup retains native preparation and capacity.
Those fixtures still use the signed package and real durable writer. Two decoder
fixtures cover strict fields and the shared SDK snapshot through actual request
decoding and response encoding. These kernel fixtures are currently drafted,
not compiled or executed. Separately, 19 shared peer transport tests and one
publication unit pass, including queued expiry, capacity retention, immediate
next reads, incomplete-frame cancellation and preservation of the physical ACK
when cancellation wins cleanup. The SDK suite passes 44 tests,
including six HTTP regressions. Private loopback and fixed transport fixtures do
not authorize a production loopback connection or prove TLS acceptance.

Primary API evidence:

- [Pinned Hickory package and MSRV](https://github.com/hickory-dns/hickory-dns/blob/v0.26.2/Cargo.toml)
- [Pinned resolver features](https://github.com/hickory-dns/hickory-dns/blob/v0.26.2/crates/resolver/Cargo.toml)
- [Pinned Apple DNS configuration](https://github.com/hickory-dns/hickory-dns/blob/v0.26.2/crates/resolver/src/system_conf/apple.rs)
- [Hyper handshake over caller-owned I/O](https://docs.rs/hyper/latest/hyper/client/conn/http1/fn.handshake.html)
- [Tokio connected peer address](https://docs.rs/tokio/latest/tokio/net/struct.TcpStream.html#method.peer_addr)
