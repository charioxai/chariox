# Chariox connection patch (MP-08 / MP-10)

This is the published ureq 2.12.1 crate source, with its Apache-2.0 and MIT licenses retained. The workspace pins this copy through `[patch.crates-io]`.

Upstream `stream::connect_host` uses the configured connect timeout instead of the caller deadline and consumes `Resolver::resolve` once. `Request` keeps its timeout private; `MiddlewareNext` is single-use and owns the request payload. Middleware therefore cannot safely implement a connection-only retry that preserves deadlines and arbitrary bodies.

Behavior changes are confined to `src/stream.rs` and `src/resolve.rs` (a smoke-test whitespace line and obsolete parentheses in `src/middleware.rs` are also normalized for current compiler checks):

- Use the earlier of the request deadline and connect timeout, including time already spent resolving or redirecting.
- Give an address half the remaining connect budget when another address or a pending family is available.
- After failed TCP attempts, ask the resolver for untried answers using the original deadline. Default resolvers return none; the kernel resolver waits only on its already-running lookup. A response that arrived between the initial list and the pending check also counts as available.

These callbacks run before HTTP headers or body transmission. They do not replay HTTP requests, change proxy selection, or change TLS hostname/certificate validation. Entry-point regressions live in the kernel's `outbound_dial` tests and use real listening and unanswered TCP endpoints.
