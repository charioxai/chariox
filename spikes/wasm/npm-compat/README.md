# npm packages in JS WebAssembly components

Can a JS App pull in ordinary npm packages and still build into a component
that plain `wasmtime` runs? Two toolchains were tested on b3 on 2026-10-22:

- **QuickJS**: `jco componentize --backend quickjs` (componentize-qjs 0.4.5), async world `npm-async`.
- **StarlingMonkey (SM)**: `jco componentize`, the default backend (componentize-js 0.23.0), sync world `npm`.

Other tools: jco 1.39.0, Rolldown 1.2.15, wasmtime 50.0.0 CLI and Node 24.20.0.
Packages are at their latest versions: zod 4.6.7, date-fns 4.4.0, lodash-es 4.18.1,
nanoid 6.0.2 and marked 18.1.1.

The files here are `npm.wit` (the worlds), `entries/*.js` (one entry per package, plus
`bench.js`) and `run.mjs` (build, run and compare).

## Compatibility

Each entry was bundled with plain Rolldown (`--platform browser`, nothing injected),
componentized, run with `wasmtime run --invoke 'run()'`, and its output compared with
the same entry run in Node (`TZ=UTC`). Build time is the wall time of `jco componentize`.

| Package | QuickJS: build / run / same as Node | QuickJS size, build time | SM: build / run / same as Node | SM size, build time |
|---|---|---|---|---|
| zod | yes / yes / yes | 2,982,437 B, 0.28 s | yes / yes / yes | 13,848,681 B, 1.62 s |
| date-fns | yes / yes / yes | 2,059,771 B, 0.23 s | yes / yes / yes | 12,300,634 B, 1.50 s |
| lodash-es | yes / yes / yes | 2,183,838 B, 0.23 s | yes / yes / yes | 12,384,245 B, 1.54 s |
| nanoid | yes / **no**: `crypto is not defined` | 1,641,283 B, 0.21 s | yes / yes / yes | 11,997,282 B, 1.53 s |
| marked | yes / yes / yes | 2,376,869 B, 0.25 s | **no**: `SyntaxError: invalid class property name in regular expression: [\p{L}\p{N}]` | - |

The same entries built with `jco componentize --bundle`, which uses jco's own Rolldown
setup:

| Package | QuickJS | SM |
|---|---|---|
| zod | builds (7,983,256 B, 1.02 s); won't run: imports `jco:node/http`, `timers` and `console` | yes / yes / yes (14,141,640 B, 1.60 s) |
| date-fns | builds (2,362,232 B); won't run: imports `jco:node/console` | yes / yes / yes (12,399,315 B) |
| lodash-es | **build fails**: `deps/jco-node-0.1.0: no 'package' header was found` | yes / yes / yes (12,479,522 B) |
| nanoid | builds (2,357,713 B); won't run: imports `jco:node/console` | yes / yes / yes (12,377,103 B) |
| marked | builds (3,007,462 B); won't run: imports `jco:node/console` | yes / yes / yes (13,147,368 B) |

Causes of the failures:

- **QuickJS has no Web Crypto.** QuickJS-ng has no `crypto.getRandomValues`, and nanoid's
  browser build calls it. Any package that needs secure randomness needs a host-backed
  `crypto` shim.
- **SM rejects `\p{…}` in regexes.** StarlingMonkey's regex parser fails on
  Unicode property escapes during Wizer pre-initialization, so the build fails. That
  points to a build without Unicode property data. `jco --bundle` gets around this with
  its engine-compat plugin, which expands `[\p{L}\p{N}]` into explicit code-point ranges.
  That is why marked works with `--bundle` on SM.
- **`jco --bundle` on QuickJS swaps Node globals for host imports.** `console`, timers
  and `node:http` become `jco:node/*` imports, which only jco's own host provides.
  wasmtime refuses to link them. To run under wasmtime, or our kernel, bundle without
  those adapters.
- **jco bug with only the timers adapter.** When the timers adapter is the only one
  injected (lodash-es), jco writes `deps/jco-node-0.1.0/timers.wit` without a `package`
  header, so its own WIT parse fails.
- **jco edits the WIT you pass it.** It adds the `jco:node/*` imports and a `deps/` folder
  to the WIT file in place. `run.mjs` gives each build a fresh copy of the WIT.

Run time: one wasmtime process with a cold compile cache took about 0.15 s for
QuickJS and 0.75 to 0.97 s for SM. Most of that is Cranelift compiling the 12 to 14 MB
SM component. Both toolchains snapshot the initialized JS with Wizer.

## Speed and memory (`entries/bench.js`)

`compute` runs 1,000,000 iterations of a Map counter plus `Math.imul` hashing. `json`
builds 10,000 records and stringifies them, then does 5 rounds of parse, filter, map and
stringify. Times are measured inside the guest and given as `Date.now` ms /
`performance.now` ms (both are available in all three). The results match on every run
(`1000/1000/2028822816` and `626781/150813`). Bench component sizes: QuickJS 1,639,717 B
(0.22 s build), SM 11,995,547 B (2.14 s build).

| Run | QuickJS compute | SM compute | Node compute | QuickJS json | SM json | Node json |
|---|---|---|---|---|---|---|
| 1 | 195 / 194.1 | 232 / 232.6 | 34 / 33.9 | 88 / 88.2 | 38 / 37.9 | 23 / 22.4 |
| 2 | 209 / 208.9 | 231 / 231.4 | 38 / 37.5 | 91 / 90.8 | 37 / 36.6 | 23 / 23.2 |
| 3 | 207 / 207.4 | 236 / 235.8 | 36 / 35.7 | 89 / 88.7 | 36 / 36.4 | 22 / 21.6 |

Peak RSS of the whole process for the `compute` run (`/usr/bin/time -v`), with
process wall time:

| Run | QuickJS (wasmtime) | SM (wasmtime) | Node |
|---|---|---|---|
| 1 (cold compile) | 185,712 KB, 0.33 s | 691,960 KB, 0.98 s | 55,876 KB, 0.05 s |
| 2 (cached) | 25,748 KB, 0.22 s | 91,444 KB, 0.29 s | 56,072 KB, 0.06 s |
| 3 (cached) | 25,628 KB, 0.22 s | 92,008 KB, 0.28 s | 55,208 KB, 0.06 s |

Summary: both engines are interpreters, about 6x slower than Node's JIT on compute.
QuickJS is a little faster than SM on Map and integer work. SM is 2.4x faster than
QuickJS on JSON. QuickJS components are about 5x smaller and build about 7x faster.
Run 1 includes wasmtime's Cranelift compile. Runs 2 and 3 use its compile cache in
`~/.cache/wasmtime`.

## Reproduce

```sh
export PATH=/var/tmp/wasmspike/node/node-v24.20.0-linux-x64/bin:/var/tmp/wasmspike/bin:/var/tmp/wasmspike/js/node_modules/.bin:$PATH
export NODE_OPTIONS=--max-old-space-size=4096
mkdir -p /var/tmp/wasmspike/npm/wasmtime && cd /var/tmp/wasmspike/npm
curl -sSL https://github.com/bytecodealliance/wasmtime/releases/download/v50.0.0/wasmtime-v50.0.0-x86_64-linux.tar.xz | tar xJ -C wasmtime
npm init -y && npm install zod date-fns lodash-es nanoid marked
node /root/work/agent-wasmspike/spikes/wasm/npm-compat/run.mjs compat   # writes compat.json
node /root/work/agent-wasmspike/spikes/wasm/npm-compat/run.mjs bench    # writes bench.json
```

For each package and toolchain, `run.mjs` runs these steps:

```sh
rolldown src/<pkg>.async.js --format esm --platform browser -o out/<pkg>.async.bundle.js
jco componentize --backend quickjs -w npm.wit -n npm-async -o out/<pkg>.qjs.rolldown.wasm out/<pkg>.async.bundle.js
jco componentize -w npm.wit -n npm -o out/<pkg>.sm.rolldown.wasm out/<pkg>.bundle.js
jco componentize [--backend quickjs] --bundle -w npm.wit -n <world> -o out/<pkg>.<tool>.jcobundle.wasm src/<pkg>[.async].js
wasmtime run -W component-model-async=y -S http --invoke 'run()' out/<pkg>.<tool>.<bundler>.wasm
```

SM components import `wasi:http`, so they need `-S http`. `<pkg>.async.js` is a
generated two-line wrapper that exports `async run()`.
