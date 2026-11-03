# Async App contract 1.0

MP-08 / MP-09 / MP-10 / MP-11 — Apps v2 R1, implementing
[Apps v2 §4.5](../../spikes/wasm/APPS_V2_PLAN.md#45-wit-versioning-and-library-compatibility).

The production worlds are exactly `chariox:app/app@1.0.0` and
`chariox:app/migration@1.0.0`. SDK bindings use `wit/app.wit`.
`wit/admission.wit` is the kernel's import/export policy data. Its admission
worlds add only the enumerated WASI 0.2.0 compatibility interfaces. Vendored
packages contain other interfaces so upstream definitions resolve; their
presence does not allow importing them. No namespace or version wildcard is
admitted. Adding a compatibility import is a reviewed policy change.

`check_component(bytes, exact_world)` checks the actual binary, not an embedded
producer claim or custom WIT section. It inspects every outer import/export,
including core module and component imports that WIT decoding can omit,
checks allowed interface members, validates the binary, and structurally checks types with pinned wasm-tools
0.262.0. Unknown imports return `ComponentError::ImportNotAllowed { import }`
with the exact binary name. Unknown exports, missing handlers, forged types,
sync downgrades, malformed files and unsupported exact worlds are refused
before instantiation. Nested components may link their own internal interfaces;
only the outer imports can obtain host authority.

The kernel installer, worker `check` and future Library preflight must use the
same function. WIT admission does not establish signature trust, resource policy,
kernel features, worker availability or OS containment; all remain required by
the package verifier and runtime admission path. `chariox_app_package::v2::verify` calls this checker on the signed backend and
every declared migration step after payload integrity checks, before returning
`VerifiedPackageV2`. Package failures use the existing `IncompatibleContract`
code and name the component path and offending import. No kernel protocol
allocation is needed for the offline check.

| Interface | Stage 1 consumers | Purpose |
|---|---|---|
| `app-types` | R2/R3, L1/L2, A1, D1 | Typed JSON, actor/context, errors, occurrences and wakes |
| `registry` | R2/R3, L1/L2 | Seal declared tools/events/lifecycle before readiness |
| `state` | R2/R6, L1/L2, D1 | Transactional installation state and migrations |
| `schedule` | R2/R3, L1/L2 | Durable wakes |
| `events` | R2/R3, L1/L2 | Durable receipts and streamed live events |
| `connections` | R2/R6, L1/L2 | Actions on user-granted connections |
| `files` | R2/R6, L1/L2, D1 | Atomic writes, snapshots, import/export grants |
| `http` | R2/R6, L1/L2 | Brokered streaming HTTP |
| `log` | R3, L1/L2 | Bounded worker logging |
| `host` | R2/R6, L1/L2 | Notifications, links, clipboard and file choice |
| `validation` | R2/R6, L1/L2, A1 | Existing protected-effect approval operations |
| `handlers` (export) | R2/R3, L1/L2, A1, D1 | Concurrent dispatch, actor/context and cancellation |
| `migration-handlers` (export) | R3/R6, L1/L2, D1 | State-only migration with deadline/cancellation |
| WASI 0.3 clocks/random/filesystem/CLI/HTTP | R3/R6, L1/L2 | Standard APIs virtualized onto scoped brokers |
| Explicit WASI 0.2.0 compatibility set | R3/R6, L1/L2 | Toolchain runtime adapters with the same scoped brokers |

Only local registration, pure occurrence IDs and queued logs are synchronous.
Every kernel request is async. Every cancellable supervisor dispatch, including
lifecycle, wake and migration calls, carries a deadline context and a cancellation
future. The future resolves with a reason on cancellation/deadline and is closed
when normal work ends. This is cooperative cancellation; it does not promise
that dropping a JS Promise cancels the underlying component task.

HTTP bodies, atomic file contents and live subscriptions are streams with bounded
worker buffers and backpressure. WASI filesystem descriptors, HTTP
requests/responses and WASI 0.2 I/O handles are owned resources with destructors.
R2/R3/R6 must associate resources with their admitted call/installation, release
broker streams on EOF/drop/cancel/trap, and retain ownership of an abandoned
pending import until it actually settles. Exceeding the cancellation grace period
requires worker termination/restart, rather than freeing resources still used by
a live import. These are runtime obligations, not claims proved by WIT types.
R6 must link WASI 0.2 and 0.3 HTTP through the existing capability broker;
linking an ambient outbound HTTP implementation is forbidden. R6 must also
supply only installation data/tmp filesystem preopens and keep descriptor
operations within those roots. These are activation/linker requirements: WIT
admission alone cannot enforce grant scopes or preopen contents. Migration admission exposes only `app-types`, `state`
and the migration export; it cannot obtain network/files/host capabilities.

## Version compatibility

Manifests name an exact world, never `1.x` or an SDK range. Production currently
registers 1.0.0 only. A future minor must preserve older imports and add only
`@since(version = 1.N.0)` interfaces/functions. Registration requires both a
structural compatibility vector and an explicit update to the supported-world
and versioned import policy. An unregistered 1.1 world remains unsupported even
when its interfaces happen to fit.

Changing/removing a function, resource, record field, parameter, result or
async status requires a major. Before releasing major 2, publish its replacement
path and a deprecation notice. Support major 1 and major 2 side by side for at
least 180 days after major 2's stable release, with at least 180 days' advance
notice before removing major 1. Maintain both contracts on the patched worker;
never retain a vulnerable engine to preserve an AOT artifact. No second major
is supported by this change.

## Source-built compatibility vectors

Tests build tiny executable component fixtures from checked-in WIT using exact
`wit-component`, `wit-parser`, `wasmparser`, `wasm-encoder` 0.262.0 and `wat`
1.262.0 from the workspace lockfile. The core implementations are generated trap
stubs from upstream `dummy_module`, with the async callback ABI. No opaque
component binary is checked in and no fixture code executes during admission.

- `tests/fixtures/v1.wit`: async state/HTTP, streams, cancellation and handlers;
  accepted for production 1.0.
- Full admission-world fixture: every admitted interface including real WASI
  resource signatures; accepted.
- `tests/fixtures/v1.1.wit`: test-only additive `@since` interface. Both the old
  1.0 component and the new component pass the extended policy. Production
  rejects its unregistered exact world and its new import.
- `tests/fixtures/incompatible.wit` and source mutations: changed parameters,
  changed exports and sync downgrades are refused despite matching names.
- Additional hostile vectors cover sockets, unlisted WASI patch versions,
  incoming HTTP, hidden core-module imports, exports and malformed binaries.

Run through the lane's cargo slot wrapper:

```sh
cargo test --locked -p chariox-app-wit
```

These prove offline component admission, not a working worker, OS sandbox,
public-site UI flow or hosted parity. Real installation/worker drills require
the R3/R4/R6 consumers and remain part of their integration acceptance.

Wasm-tools 0.262's fixture generator includes `stream.forward`/`future.forward`
canonical intrinsics. This WIT validator enables `WasmFeatures::CM_FORWARD`
explicitly, alongside parser defaults. R3 must independently check engine feature
support before activation; the Wasmtime 50 spike has no public forward-feature
configuration setter, so these fixture bytes are not evidence of a working
Wasmtime 50 worker. The exact production worker/toolchain combination must pass
its own compile/link/run drill. Do not equate this syntax/type check with engine
availability.


## View callers and forms

The browser bridge supplies a caller to each UI action handler:

```js
chariox.action(declaration, async (input, caller) => {
  return caller.call('save', input);
});
chariox.form(document.querySelector('form[data-chariox-action]'), async (input, caller) => {
  await caller.call('save', input);
  await caller.call('refresh');
});
```

A caller belongs to one handler invocation and expires when its returned promise
settles. The controller binds its opaque call-id token to the invoking agent and
view; the page supplies no actor identity. Retired or unknown tokens fail with
`APP_INVOCATION_EXPIRED`. Ambient `chariox.call` always acts as the human.
A human submission and an agent invocation both run the registered form handler.
An annotated form without `chariox.form` registration fails with
`form_unregistered`; native async submit listeners cannot signal completion.
Another agent action on the same Tab gets `view_busy` until the handler settles,
including after a deadline. Human input remains available.

The WIT `handlers` exports are backend entry points, distinct from browser UI
handlers. JS/TS, Rust guest, C and C++ backend handlers retain their existing
kernel-supplied context and signatures. Signed declarations and transport shapes
are unchanged. Use the browser caller for backend calls made by a view handler;
do not replace backend WIT parameters with a browser caller.
