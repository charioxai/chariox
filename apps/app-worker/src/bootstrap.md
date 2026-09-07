# Trusted App backend bootstrap

An App exports one registration function, either an ES module default export or
CommonJS `module.exports`. The function receives the frozen `AppBackendSdk` and
may return a promise. It registers every tool/event declared by the verified
package and any optional lifecycle handlers. The bootstrap then seals those
registrations through the existing `worker.ready` request. It owns `ready`,
`close`, and FD3; the injected App API omits those transport-lifecycle methods.
Scaffolding should generate `runtime/main.mjs` with `export default function
register(chariox) { ... }` and let the existing packer choose that entry.

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
  declarations: { tools: ['list_issues'], events: [] },
  startupTimeoutMs: 10000,
});
```

`/runtime` is the fixed Linux mount. macOS substitutes its verified immutable
runtime path. The kernel encodes paths/configuration as JSON data in this small
script and enforces the existing 256 KiB native bootstrap envelope. It never
inserts executable App source or accepts this configuration from a terminal.
The entry and declaration allowlists come from the verified package; the kernel
remains responsible for input/output schemas and operation authorization.

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
132 startup timeout, 133 IPC failure, 134 uncaught exception/rejection, and 135
shutdown/flush failure. These are worker exit categories, not new wire messages.

`node --test --test-concurrency=1 apps/app-worker/tests/bootstrap.test.mjs`
uses ordinary Node processes and real inherited FD3 streams. It proves bootstrap
registration, framing, dispatch, bounded errors, shutdown flushing and teardown;
it does not prove native confinement or embedded Node compatibility. The native
CI workflow separately runs Linux containment and then the development macOS
Seatbelt probe. Signed/hardened macOS validation remains required, including the
documented unsigned file-map-to-executable observation.

Primary reference: [Node 24.20 embedder entry and createRequire](https://github.com/nodejs/node/blob/v24.20.0/doc/api/embedding.md).
