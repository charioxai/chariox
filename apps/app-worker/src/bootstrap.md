# Trusted App backend bootstrap

An App exports one registration function, either an ES module default export or
CommonJS `module.exports`. The function receives the frozen `AppBackendSdk` and
may return a promise. It registers every tool/event declared by the verified
package and any optional lifecycle handlers. The bootstrap then seals those
registrations through the existing `worker.ready` request. It owns `ready`,
`close`, and FD3; the injected App API omits those transport-lifecycle methods.
Scaffolding should generate `runtime/main.mjs` with `export default function
register(chariox) { ... }` and let the existing packer choose that entry.

Before any App module loads, the bootstrap replaces every `node:fs` fsync and
fdatasync form with one that fails `ERR_ACCESS_DENIED`: the sync and callback
forms, which Node 24.20's permission model already denies, and the `FileHandle`
methods, which it lets through. Node's own guard varies across releases (some
deny none, newer ones deny all), so this bootstrap is the only deny that stays
stable across runtime updates. It then re-syncs
the built-in ESM exports. The
SDK README's "Durability" section is the contract. This applies to the App's
own thread only. A worker thread gets its own `node:fs` and keeps Node's
behavior. It cannot be patched reliably: a `Worker` started with its own
`execArgv` does not even inherit `--permission` on Node 24.20, which is why the
OS sandbox, not Node's permission model, is the containment. If the deny's
probe of its own file fails, the worker exits 131, like an App import failure.
That file is readable wherever this one could be loaded.

This source is a runtime artifact, never an App-selected module. The release
bundle must contain `bootstrap.cjs`, `bootstrap-config.cjs`, and the exact
`@chariox/app-sdk` package source graph at `sdk/`, alongside the native runtime
libraries. The artifact verifier and immutable runtime lease cover every file.
Current native-library build tooling does not yet assemble/sign that full bundle.

After native confinement and the FD4 Ready/Continue exchange, the kernel's
serialized launch bootstrap uses Node's documented built-in-only entry `require`
to obtain the ordinary file loader:

```js
require('node:module').createRequire('/runtime/bootstrap.cjs')(
  '/runtime/bootstrap.cjs'
).start({
  version: 1,
  entry: 'runtime/main.mjs',
  declarations: { tools: ['list_issues'], incomingEvents: [] },
  startupTimeoutMs: 10000,
  // Only for an update raising the data schema version; both keys or neither.
  migrations: [{ from: 1, to: 2, entry: 'migrations/002.js' }],
  migrationTimeoutMs: 60000,
});
```

`/runtime` is the fixed Linux mount. macOS substitutes its verified immutable
runtime path. The kernel encodes paths/configuration as JSON data in this small
script and enforces the existing 256 KiB native bootstrap envelope. It never
inserts executable App source or accepts this configuration from a terminal.
The entry and declaration allowlists come from the verified package; the kernel
remains responsible for input/output schemas and operation authorization.
`incomingEvents` contains only signed events with direction `incoming` or `both`.
An outgoing-only declaration never requires a JavaScript handler or appears in
the readiness event list. Old ambiguous `declarations.events` input is rejected.

`migrations` is the consecutive pending chain from the installation's data
version to the release's `migrations.targetVersion`; each entry obeys the same
containment rules as the App entry under `migrations/`. The bootstrap runs each
step's default export with the SDK's migration context, reports `migration.step`
and only then imports the App entry. `migrationTimeoutMs` (at most 120 seconds)
bounds the whole migration phase; `startupTimeoutMs` is re-armed for App load.

The bootstrap captures native-filtered `CHARIOX_APP_GENERATION`,
`CHARIOX_APP_INSTALLATION`, `CHARIOX_APP_RELEASE_DIGEST`, `CHARIOX_APP_PACKAGE`,
`CHARIOX_APP_DATA` and `CHARIOX_APP_TMP`. It checks identity syntax, canonical
disjoint roots and strict entry containment before importing App code. Its fixed
relative SDK imports resolve from the trusted runtime. It uses Node's normal
module loader for ESM/CommonJS and their packaged dependencies. Native library
loading, subprocesses and network authority remain governed by the outer sandbox.

The existing `lifecycle.dispatch` shutdown call runs the App's optional shutdown
handler. Its response is flushed through FD3 before the bootstrap closes the
channel and exits. Failed/timed-out shutdown exits with failure after its bounded
reply; a one-second flush ceiling prevents a disconnected reader from stranding
the process. Channel loss and fatal exceptions terminate the worker even when
the App has live timers. Timers bound cooperative JavaScript startup; the kernel
supervisor must still terminate synchronous loops and enforce native resources.
No script-level mechanism is claimed as containment or as authority over hostile
code sharing the JavaScript process.

Failure output is only `app_worker_bootstrap_failed:<code>`; exception text,
paths, bootstrap/configuration bytes and IPC payloads are not copied to errors.
Codes are 130 invalid configuration, 131 import/registration/readiness failure,
132 startup or migration timeout, 133 IPC failure, 134 uncaught exception/rejection,
135 shutdown/flush failure, and 136 data migration failure. These are worker exit categories, not new wire messages.

`node --test --test-concurrency=1 apps/app-worker/tests/bootstrap.test.mjs`
uses ordinary Node processes and real inherited FD3 streams. It proves bootstrap
registration, framing, dispatch, bounded errors, shutdown flushing and teardown;
it does not prove native confinement or embedded Node compatibility. The native
CI workflow separately runs Linux containment and then the development macOS
Seatbelt probe. Signed/hardened macOS validation remains required, including the
documented unsigned file-map-to-executable observation.

Primary reference: [Node 24.20 embedder entry and createRequire](https://github.com/nodejs/node/blob/v24.20.0/doc/api/embedding.md).

SDK 0.7 installs a fixed global `fetch` from the broker SDK before importing App
code. It preserves native Web value objects and routes every Fetch network
operation over FD3; the inherited-channel test replaces ambient Fetch with a
trap and completes a gzip exchange through the broker messages. Bootstrap/SDK
changes belong to the versioned bundle graph, not the native compiler inputs.
See [the Fetch contract](../../../packages/app-sdk/FETCH.md) for its precise
supported subset and remaining integrated validation.

On macOS the bootstrap also replaces `fs.watch` and `fs/promises` `watch` for
directories before any migration or App module loads (`syncBuiltinESMExports`
covers ESM imports). Node watches a macOS directory through FSEvents, which the
Seatbelt policy denies (see the launcher's platform policy), so libuv reported
an asynchronous `EMFILE` and an App without an `error` listener exited. The
replacement lists the directory (recursively when asked) every 250 ms and
reports `rename`/`change` from inode, size and time stamps; more than 4096
entries fail with `ENOSPC`. Non-directories keep Node's own watch (kqueue on
macOS), and Linux keeps inotify. This is compatibility, not containment: the
listing uses the same permission-checked `node:fs` calls as the App.

Before the SDK or any App module loads, the bootstrap also keeps worker threads
inside Node's permission model. On Node 24.20 a `Worker` given its own
`execArgv` (even `[]`) is parsed afresh, so it starts without `--permission` or
with wider allow-lists; a `NODE_OPTIONS` in its environment is parsed too; and
`module.register` starts a hooks thread that inherits `--allow-worker`. So every
`Worker` gets the launcher's `--permission`, `--no-addons` and file allow-lists,
without `--allow-worker` (a worker thread cannot start another one). An App's
`execArgv` may only repeat launcher flags, `NODE_OPTIONS` is dropped from the
worker's environment, `SHARE_ENV` is refused, and `module.register` fails with
`ERR_ACCESS_DENIED`, as Node itself does without `--allow-worker`. The
replacement `Worker` is a plain wrapper function (not a Proxy, which
`util.inspect` could unwrap), so the native constructor is not reachable
from the export, its prototype or an instance. The instance's native thread
handle, and the same handle published on the `worker_threads` diagnostics
channel, still expose the handle's own constructor, which is the one reachable
way to build a fresh handle that can start a thread (a handle faked with
`Object.create` has no native state and its `startThread` throws). So the guard
neutralizes that constructor on the shared handle prototype during setup, before
any App or migration code runs and could capture the original; a reference an
App reads afterwards, from an instance or the channel, is the neutralized one,
and the original is unreachable. Child processes, WASI, the
inspector and `process.binding` stay denied by Node's permission model. Node's
permissions remain defense in depth; the native sandbox contains the worker.
