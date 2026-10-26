# App component builds (MP-08 / MP-09 / MP-10 / MP-11)

These source builds target R1's async `chariox:app/app@1.0.0`. They produce the
portable backend for `chariox.app.v2`; `app-package::v2::pack` and `verify` remain
the signing and package authority. There is no new host function or protocol.

## Build

Use Rust **1.99.1**, its `wasm32-wasip2` target, Python >=3.11 with tar extraction
filters, Node 22, and the repository's pnpm lock. Put scratch and outputs outside
the checkout. The lock contains checksummed Linux/macOS x86_64 and ARM64 assets.
Linux x86_64 is exercised here; native macOS builds and live validation remain
integration gates. The emitted component has no OS-specific host interface.

```sh
export TMPDIR="$HOME/.chariox/dev/my-app/tmp"
mkdir -p "$TMPDIR"
rustup target add wasm32-wasip2
pnpm install --frozen-lockfile --filter @chariox/app-sdk-tools
python3 packages/app-sdk-tools/build-toolchain.py "$HOME/.chariox/dev/my-app/compiler"
export CHARIOX_APP_TOOLCHAIN="$HOME/.chariox/dev/my-app/compiler"
cargo build --locked -p chariox-app-sdk-tools
export CHARIOX_APP_COMPONENT_CHECK="$PWD/target/debug/chariox-app-component"
node packages/app-sdk-tools/build-js.mjs packages/app-sdk-js/template/app.ts "$TMPDIR/app.wasm"
```

On shared builders pass `--cargo-runner /path/to/cargo-slot.sh` to the Python
builder, and run every other Cargo command through that same wrapper. Copy the
checker out of a shared target while still holding its build slot.

Rust (run the Python builder inside the Cargo slot on a shared builder):

```sh
python3 packages/app-sdk-tools/build-rust.py packages/app-sdk-rust/template/Cargo.toml "$TMPDIR/app.wasm"
```

The checker with one argument performs admission only. With an output argument
it first prepares toolchain compatibility, then runs that same R1 admission.
Never distribute the raw compiler output in place of the prepared component.
`build-js.mjs` does this automatically and preserves an existing output on errors.
Its external imports are WIT interfaces; `node:*` builtins fail during bundling.
Actual component imports are checked after compilation, including npm dependencies.
The packer uses the source-built QuickJS CLI; the npm QuickJS release and jco's
Node-adapter bundling path are not a second production toolchain.

The Wizer patch freezes build-time randomness and clocks, so snapshots are
reproducible. Runtime host randomness is unchanged. Generate random values and
timestamps inside handlers, rather than freezing them in module initializers.
Runtime and bundled npm dependency notices are embedded in the component's
`chariox:licenses` custom section and therefore enter the signed payload.
MPL dependencies retain their bundled source files there. The generated
toolchain inventory records permissive choices for dual-licensed dependencies.
The host compiler is built locally and is not distributed by this SDK. Its
`json-escape-simd` dependency declares MIT but supplies no notice text; resolving
that upstream gap is required before distributing compiler binaries.

The Rust builder preserves dependency and standard-library notices too (install
the pinned toolchain's `rust-docs` component when those texts are absent). It
normalizes only the standalone App crate's metadata salt and source prefix;
dependency crate identities remain Cargo's responsibility. SDK dependencies come
from the same fixed SDK checkout in the reproducibility drill.

## Compatibility and resources

The pinned Rust std and preview1 adapter emit WASI 0.2.12. Preparation maps their
outer interface names to the admitted 0.2.0 names and **checks the complete
resulting structural types and members**. It does not admit new WASI members.
Ambient CLI environment, stdin, terminal and process-exit imports are linked to
trap implementations inside the component. Code attempting those facilities
traps. Legacy insecure RNG imports are implemented using the already admitted
secure random interface. Clocks, stdout/stderr and data/tmp operations retain
their existing host interfaces. Sockets remain refused with R1's typed error.

JS handlers receive `ctx.signal`, `ctx.fetch`, `ctx.subscribe` and
`ctx.resources`. Use these scoped APIs for concurrent calls. Scope exit drops
unread streams, requests QuickJS import cancellation, and disposes any late
response that wins a cancellation race. Cancellation futures are cancelled,
awaited, then dropped on **all** exits, including normal success.

Raw WIT imports and standalone `fetch` are also available for explicit ownership.
When using standalone fetch, consume its body, call `body.cancel()`, or pass an
AbortSignal. Global timers must be awaited or cleared in their originating
invocation. They are not durable App wakes; use `schedule` for that. UTF-8
TextEncoder/TextDecoder, AbortController, Headers, bounded fetch responses,
console, microtasks, timers and random values are provided. The response body is
an async iterable, not a complete browser ReadableStream implementation. Buffered
response helpers default to 1 MiB; iterate `body` for larger results. Other text
encodings, DOM, Node APIs, relative URLs and WebCrypto algorithms are unsupported.

Rust uses generated WIT ownership and `cancellable`, which drops the losing work
future and its owned streams. It preserves all u64 values as u64; JS uses bigint.

## Evidence and integration boundaries

`test-components.sh` rebuilds both templates in different source directories,
compares exact bytes, signs packages and calls `app-package::v2::verify`. It also
runs the actual components under plain Wasmtime, exercising shared state and
QuickJS endpoint cleanup. Run under a Cargo slot with `CARGO_TARGET_DIR` set by
that wrapper; the script deletes its own scratch on exit.

These are compiler/ABI/resource regression checks. They do not establish kernel
broker cancellation, installation, sandboxing, macOS execution, hosted-client
behavior or ordinary/managed parity. R3 supplies the actual worker, R4 the
installer, R6 the broker-backed WASI interfaces; their real kernel/client drills
are required before Stage 1 acceptance. No MP item is closed by these checks.

Each template includes `app.json`, tool schemas and a minimal view entry. Place
the compiled component at `bundle/runtime/app.wasm` before packaging. Replace
`com.example` and the publisher metadata with the normal registered publisher;
the templates contain no signing key. The blank view is a contract placeholder
for the view SDK, not an Excalidraw/Univer or full Todo UI implementation.
