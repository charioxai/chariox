# Chariox Apps v2: the shape (appshape r1, 2026-10-22)

For Miguel. Design only; no product code changed. The one piece of code is
the measured C test in `spikes/wasm/apps/c-app/` (§6). Paths are relative
to the OSS repo at branch `wasmspike/r3`; numbers come from
`/root/.codex/evidence/browser-resume-20260930/appshape/r1/` and the r2
spike (`spikes/wasm/DECISION.md`).

How to read it: each section describes one part of the App, then ends with
**Consequences** and **Open points for you**. §8 collects every decision.

Binding decisions already taken (10-22) and used as given here: WASM
components on wasmtime in a Rust worker, async from the start, QuickJS for
JS/TS, one process per App, `engine: "node"` packages are rebuilt, `/app dev`
is the dev shape with a local register step, and agent actions are declared
in the UI.

---

## 1. The App today, end to end

### 1.1 Backend

An App backend is JavaScript running in an embedded Node worker that the
kernel launches and sandboxes.

- The kernel spawns the worker with `posix_spawn` and fixed descriptors:
  FD3 is the private SDK socket, FD4 the startup control channel, FD1/2
  bounded log pipes (`apps/app-worker/src/launcher.md:49-55`;
  `packages/app-runtime/src/worker_process.rs:330`). The environment is
  clean except `HOME`, `TMPDIR` and `CHARIOX_APP_*`; there is no token, the
  socket itself is the authority (`launcher.md:23-27`).
- Linux: pinned bubblewrap with user/pid/net/ipc/uts/cgroup namespaces, a
  seccomp filter, cgroup limits (`packages/app-runtime/src/worker_process/platform_linux/plan.rs:51-60`,
  `apps/app-worker/src/sandbox_linux.c:10-30`, `platform_linux/policy.rs:41-50`).
  macOS: Seatbelt `(deny default)` plus a memory monitor
  (`apps/app-worker/src/sandbox_macos.c:14-19`, `worker_process.rs:134-136`).
- Limits: 512 MiB memory, 64 tasks, 512 MiB private data, 15 s startup,
  30 s per call (`worker_process.rs:136,201-230`, `private_data.rs:18`).
- `bootstrap.cjs` loads the pinned SDK, opens FD3, replaces `fetch` with the
  SDK's broker fetch, runs migrations, calls the App's `register(chariox)`
  and sends `worker.ready` (`apps/app-worker/src/bootstrap.cjs:303-365`).
- Concurrency caps stack up: 64 kernel→worker calls, **16 worker→kernel
  broker calls** (`BUSY` beyond), **16 SDK handlers** (`APP_BUSY`), 8
  kernel-wide App admissions, 4 live workers
  (`packages/app-runtime/src/worker_peer.rs:64-65`,
  `packages/app-sdk/src/peer.js:5,137-139`,
  `apps/kernel/src/runtime/app_control.rs:98`, `app_lifecycle.rs:41`).
- Workers start on demand, idle out, and a failed worker restarts after 1,
  4, 16 s, then is quarantined (`apps/kernel/src/durable_state/app_worker_lifecycle/store.rs:116-129`).

### 1.2 SDK

`@chariox/app-sdk` (`packages/app-sdk/src/index.d.ts:128-208`): `tools`,
`events`, `lifecycle`, `state`, `schedule`, `connections`, `files`, `http`,
`log`, `host`, `validation`. Every handler gets an `InvocationContext` with
deadline, room, operation, agent, task, turn and `actor: human | agent |
background` (`index.d.ts:3-20`). The wire is length-prefixed JSON frames on
FD3, 1 MiB max (`packages/app-sdk/WIRE.md:36-40`). It is explicitly not an
MCP server (`WIRE.md:3-7`).

### 1.3 Views: where and how they render

- The manifest has **one** UI entry (`ui.entry`, HTML under `ui/`) plus an
  optional `agentPanel` placement (`packages/app-package/src/manifest.rs:25,61-90,217`).
- A view is a **Tab of the kernel's browser** (Chromium). Humans and agents
  share that one DOM (`apps/kernel/src/runtime/state/app_view_runtime.rs:1-4`).
  Assets are served by CDP request interception on a per-installation
  `https://app.<label>.invalid` origin; everything else is blocked by CSP
  (`apps/kernel/slice-linux-docker/docker/browser-controller-apps.mjs:1-56`;
  `docs/PROTOCOL.md:2711-2731`). UI files are capped at 2 MiB per view
  (`PROTOCOL.md:2744`).
- The page talks to the backend through one CDP binding, `window.chariox`:
  `call(tool, input)` runs the App's own tool as the owner (or as the agent
  that drives the view), through the same schema check and durable path as
  agent calls; plus `host.openLink`, `host.writeClipboard`, `panel.set/get`
  (`browser-controller-apps.mjs:26,51-90`; `app_view_runtime.rs:818-850,900-930`).
- Clients project that Tab: the web client shows it in its browser modal,
  the TUI opens it with `/app open` as a Room Tab, and a flagged text
  (accessibility outline) view exists for the terminal
  (`packages/kernel-client/src/shell-app-command.ts:65-72`;
  `docs/MULTIDOMAIN_APP_VIEWS.md:104-108`; `docs/MULTIDOMAIN_TUI_APP_VIEWS.md:30`).
  A session-less `OpenUserAppView` returns a verified frontend bundle for
  native hosts (`packages/kernel-client/src/ipc-app-requests.ts:220-221`).
  The web client code lives in the Cloud repo, not here. Native and mobile
  viewers are Phase 2 (`docs/CHARIOX_APPS_IMPLEMENTATION_PLAN.html:1189-1212,1356`).

### 1.4 Capabilities, approvals, state, HTTP, files, events

| Part | Today (built) | Where |
|---|---|---|
| Capabilities | `network [{origin, methods}]`, `clipboard`, `externalFiles`, `workflows`, `agents`, `connections`; tools/events/actions declared as JSON schemas | `manifest.rs:97-122`, `packages/app-package/src/declarations.rs:18-64` |
| Approvals | install, update and reinstall each raise one owner interaction "…with the declared capabilities?" with a 5-minute, digest-bound challenge. Agents never answer. | `apps/kernel/src/runtime/app_install_control/pump.rs:272-299`, `…/README.md:37-44` |
| Critical actions | passkey (vault passphrase) approval through `validation.request` | `apps/kernel/src/runtime/app_validation_broker.rs:1-4` |
| State | versioned key-value with transactions: 4096 keys, 256 KiB values, 16 MiB total | `packages/app-runtime/src/managed_state.rs:18-25` |
| HTTP | exact declared origin+method only; anonymous (no credentials yet); 512 KiB buffered or streamed | `apps/kernel/src/runtime/app_http/policy.rs:170-199`, `app_http/README.md:24-27` |
| Files | `node:fs` inside package/data/tmp roots; `files.atomic_replace` 512 KiB; snapshot, import/export via file-grant prompts | `apps/kernel/src/runtime/app_files_broker.rs:74-80` |
| Events | AEDS (event delivery service) pushes provider events over WebSocket into an App **inbox**, durably stored, delivered at least once to `events.register` handlers; Apps emit through a transactional **outbox**; owner automations map events to workflows | `apps/kernel/src/transport/event_delivery_client.rs:12-14`, `packages/app-runtime/src/app_inbox.rs:1-6`, `app_outbox.rs`, `docs/EVENT_TRIGGER_PROTOCOL.md:11-18` |

Note: the dev-loop README says an update with unchanged capabilities is
approved without a prompt (`apps/cli/src/app-install-file/README.md:72-74`),
while the install-control code prompts for every update
(`pump.rs:272-299`; only previously consented deployment releases skip,
`requests.rs:224-229`). One of the two is wrong; §3 decides which we want.

### 1.5 Dev loop, package, install, generations, Library

- **`/app dev ./my-app`** (built, `apps/cli/src/app-dev-loop.ts`): watches
  the directory (500 ms debounce), packs with the installer's verifier,
  installs on the first cycle, then updates through `BeginAppUpdate`. The
  update drains the worker, snapshots state, runs migrations and a health
  check, then commits; a failed start keeps the old generation. It never
  opens or reloads views; the user reopens them (`README.md:58-76`;
  `docs/PROTOCOL.md:2775-2788`). It signs with the developer's enrolled
  Ed25519 publisher key; there is no dev or test signing mode
  (`README.md:79-91`).
- **Package** (built): `.cxapp` = canonical USTAR with `manifest.json`,
  `integrity.json`, `signatures/publisher.sig`; Ed25519 over RFC 8785
  canonical bytes; 128 MiB archive, 64 MiB file
  (`packages/app-package/README.md:136-218`). The only engine is `Node`
  (`manifest.rs:55-56`). Trust comes only from a publisher key the owner
  enrolled with a fingerprint prompt (`apps/kernel/src/runtime/app_publisher_control/README.md:1-17`).
- **Install and generations** (built): installations carry `generation`,
  `allocated_generation`, `pending_generation`; candidates are never
  reused; committed generations cannot be reverted; data (kernel SQLite
  state + a 512 MiB private volume) is per owner and installation, not per
  generation (`packages/app-runtime/src/installation.rs:180-217`,
  `packages/app-runtime/README.md:18-23`, `private_data.rs:16-44`).
  Releases live immutably in a release store and are re-verified on every
  start (`release_store.rs:29-31`; `app_lifecycle/README.md:8-12`).
- **Library, Developer Center, automatic review, TUF updater** are plan-only
  (P2.01–P2.05, `CHARIOX_APPS_IMPLEMENTATION_PLAN.html:1618-1641`). Two
  distribution routes are planned: the Library (no source upload) and a
  publisher-hosted signed feed (`:1068-1072`). Nothing of it exists in code.

### 1.6 How agents use an App today

- App tools are **MCP tools on the kernel's single runtime MCP**, not one
  MCP server per App (`apps/kernel/src/runtime/state/tool_dispatch/app.rs:1-2`).
  `tools/list` projects the agent's bound Apps as `app_<local>_<32hex>`
  tools with a 30 s timeout; critical tools are left out
  (`apps/kernel/src/runtime/app_control/catalog.rs:250-290`; `app.rs:343-351`).
- `tools/call` validates the schema, takes an admission permit, sends
  `tools.invoke` on FD3 as `Actor::Agent`, awaits the reply (cancelled if
  the binding is revoked or the turn ends) and returns it as the tool
  result the model reads (`tool_dispatch/app/invocation.rs:20-146`).
- An agent gets an App by an extension grant: explicit binding, the user
  opening the App in the foreground (binds to the focus agent), or an
  agent self-grant (`invocation.rs:305-315`; `app_foreground_runtime.rs:57`;
  `extension_request_tool.rs:55,77`).
- Agents can act on an **open view** through the Room browser share
  (`kind: "app_view"`), and view calls then carry `Actor::Agent`
  (`kernel_browser_runtime.rs:762-764`; `app_view_runtime.rs:923-929`).
  There is no agent tool that opens a view, and no agent path to the UI
  other than driving the browser.
- Events never belong to agents; they reach agents only through workflow
  triggers (`EVENT_TRIGGER_PROTOCOL.md:11`; `IMPLEMENTATION_PLAN.html:1064-1066`).

### 1.7 What the original design assumed

The plan never proposed WASM as the runtime. It assumed a **pinned Node LTS
embedded in a signed worker** (`IMPLEMENTATION_PLAN.html:837`), V8 reached
through Node, with the macOS JIT entitlement (`:974`), WASM only as a
portable npm dependency type (`:838,1167`), and the UI as "one HTML, CSS and
TypeScript implementation" in an isolated origin (`:840`). The guiding
sentence is "'Runs any App' needs the same kind of contract as 'Chromium
runs any web app'" (`docs/CHARIOX_APP_BACKEND_SANDBOX_RESEARCH.md:150`).
So an App was a web app with a Node backend, both sandboxed by the OS.

The reviews' open issues are still open: the "only reviewed code runs" claim
is not enforced since an App can fetch and `eval` text
(`docs/CHARIOX_APPS_PLATFORM_REVIEW.md:13-15`); macOS memory is a V8 heap
limit, not a process limit (`:19-21`); Chromium on the view path runs with
`--no-sandbox` (`docs/CHARIOX_APPS_PLAN_REVIEW.md:30-34`); the Node
compatibility contract is unbounded (`:110-114`).

**Consequences.** Most of the product is kernel-side and survives any
runtime change: brokers, approvals, signing, generations, the view Tab, the
runtime MCP. What is thin today is the *agent side of the UI*: agents only
get backend tools or raw browser driving, and nothing tells them what the
view can do. That is where §4 goes.

**Open points for you.** None yet; this section is the baseline.

---

## 2. What WASM changes, part by part

| Part of §1 | Kept | Changes | Lost | Better |
|---|---|---|---|---|
| Launch, sandbox, limits | FD3/FD4, bwrap/Seatbelt, cgroup, clean env | Worker is one Rust binary with wasmtime instead of embedded Node; the macOS executable-memory entitlement stays (as Node's JIT) | — | **Per-call limits** inside the process: CPU slice stops a runaway call at 1.0 s and names it; memory cap is a linear-memory cap, not a V8 heap guess (fixes PLATFORM_REVIEW P1 on macOS); 14–17 MiB RSS vs 56; cold start 2–8 ms vs 31 |
| Concurrency | one instance serves many calls (as Node's event loop) | the 16-call SDK cap disappears; the **kernel's 16 broker-call cap becomes the only ceiling** and must become a queue | — | 64 callers on one instance measured |
| SDK API | `register(chariox)` with the same namespaces for JS | implemented as a layer over WIT imports; `AbortSignal` from the `cancel` future; `BigInt` → number | `node:*` modules, npm packages that need Node built-ins or Web APIs (QuickJS has no `fetch`, `TextDecoder`, `crypto`; SDK polyfills them), 4/5 popular packages worked | the same API in Rust, C, Python, with typed contracts in one WIT file |
| Views | unchanged: the view is a browser Tab; `window.chariox.call` | nothing in the view layer depends on the backend language | — | §4 adds declared actions on top |
| Capabilities, approvals | unchanged | install also checks the component's imports against the world's allowlist (`chariox-wasm-worker-async check`) | — | the kernel can **prove** what an App can call: a component with an unknown import is refused at install, which finally makes "only reviewed code runs" true for the backend (no `eval` of fetched code in Rust/C; QuickJS `eval` stays inside the sandbox with the same imports) |
| State, HTTP, files, events | brokers unchanged | HTTP bodies and live event feeds become WASI streams with backpressure | `node:fs` → two WASI preopens (`data`, `tmp`) | 50 MiB stream at 230–280 MiB/s with +1–4 MiB memory vs Node 48 MiB/s and +37 MiB |
| Dev loop | `/app dev` unchanged in shape | pack compiles to a component (0.2–1.4 s) and the kernel precompiles at install (16–700 ms) | Node debugger / `--inspect`; `console.log` stays (stderr pipe) | one package runs on every OS and CPU; no runtime bundle per OS |
| Package, signing, install, generations | unchanged | `runtime.engine: "wasm"`, `world` field; install-time AOT cache keyed by release digest + worker build | `engine: "node"` | smaller releases (190 KiB Rust, 1.5 MB JS, 300 KB C vs Node runtime per OS) |
| Agents (runtime MCP) | unchanged | — | — | queueing lets many agents share one instance (§4) |
| Library (planned) | unchanged | the automatic review gets a component to analyse instead of a Node bundle: imports are the capability surface | — | review can be mechanical for imports |

Debugging deserves a plain answer: there is no step debugger for guest code
in wasmtime today that matches Node's inspector. What exists: source-maps
for QuickJS stack traces, `log.write`, panics with backtraces (Rust), and
the worker's per-call stop reasons. A `/app logs --follow` in dev mode is
the practical replacement at launch.

**Consequences.** The kernel work is small and already described
(DECISION.md PRs 1–6); the developer-facing losses are the Node debugger
and Node-only npm packages. The "like a web app" framing still holds for
the view and is now wrong for the backend: a backend is a typed component,
closer to a plugin than to a server.

**Open points for you.**
1. Accept the debugger gap at launch (logs + stack traces), with a proper
   debugger as a later item? *Recommend: yes.*
2. Kernel broker cap: queue instead of `BUSY`, 64 in flight per App?
   *Recommend: yes (DECISION.md Q1).*

---

## 3. Dev mode and local registration

### 3.1 Dev copy versus registered App

Today one owner has one installation per App ID and `/app dev` updates it
in place (`app-dev-loop.ts:225`). That means every save changes the App
the user and every agent actually use. You asked for a dev copy plus a
register step. Proposed shape:

| | Dev copy | Registered App |
|---|---|---|
| What it is | a second installation of the same App ID with `channel: "dev"` | the normal installation (`channel: "release"`) |
| Who sees it | the developer's own terminal and the Room where `/app dev` runs; agents **in that Room** get its tools and actions; nobody else | the user and all bound agents |
| Data | its own state and private volume, seeded on demand from the registered App (copy, not shared) | unchanged |
| Views | opened with a visible DEV badge; live-reloaded on each cycle | normal |
| Signing | a kernel-generated **dev publisher identity**, trusted only for `channel: "dev"` on this kernel, created at first `/app dev` without a prompt | the real publisher key, enrolled once with the fingerprint prompt (as today) |
| Approvals | one prompt on first dev install and whenever the capability set changes; same text as today | on register: prompt only if capabilities differ from the set the registered App already approved |
| Promote | `/app register` packs the current source with the publisher key and runs the ordinary `BeginAppUpdate` on the release installation (data kept, migrations, health check, old generation kept on failure) | — |

Why a channel and not a second App ID (`com.x.todo.dev`): tool names,
schemas, state migrations and the Library identity all hang off the App ID;
a suffixed ID would make the dev copy a different App to every agent and
would never test the real update path. A channel keeps one identity and
reuses the generation machinery unchanged.

### 3.2 Hot reload: the cost of one save, per language

"Hot reload" for now = view live-reload + backend instance swap.

| Step of one save | JS/TS (QuickJS) | C | Rust | Python |
|---|---|---|---|---|
| bundle (Rolldown) | ~0.1–0.5 s (estimate) | — | — | — |
| compile to component | 0.26 s (r2 measured) | **0.22 s (measured, 10 tools)** | 2–10 s incremental release build (estimate; not measured, the shared toolchain has no `wasm32-wasip2` target) | 1.4 s (r2 measured) |
| pack + sign + verify | <0.1 s | <0.1 s | <0.1 s | <0.1 s |
| kernel precompile (AOT) | 0.1–0.2 s | 0.016 s | 0.024 s | 0.7 s |
| update: drain, snapshot, migrations, health, commit | ~1 s today (estimate; depends on the App's `health_check`) | same | same | same |
| view reload | <0.3 s (Tab reload) | same | same | same |
| **save → new build serving** | **~2 s** | **~1.5 s** | **~4–12 s** | **~3.5 s** |

Measured items are from this round's build (`apps/c-app/build.sh`) and the
r2 build log; estimates are marked. The update path dominates for JS and C;
measuring it on the real kernel is PR D1 in §8.

### 3.3 View live-reload

Protocol 356 already defines it for Room-hosted views: on a committed
update the kernel reloads the Tab in place and answers `APP_VIEW_RELOADING`
meanwhile (`docs/PROTOCOL.md:2880-2888`). The dev loop and its README
still say "reopen the view" (`README.md:75-76`). The fix is to make the
dev loop rely on the kernel reload and to keep the view's scroll/route in
`sessionStorage` (App's job, SDK helper).

### 3.4 Keeping state across swaps

Structured state is in the kernel (SQLite tables, `durable_state/apps.rs:431-442`)
and survives every swap today; the update already snapshots it and
restores on failure. In-memory state is lost at each swap, as with a Node
restart or a wasm trap; that is the rule r2 set (durable data belongs in
`state`). Dev adds nothing here except a `/app dev --reset-data` switch
to start the dev copy from empty and a `--seed-from-release` to copy the
registered App's data again.

### 3.5 Agents in a Room developing an App

The dev loop runs **in the CLI process** (`app-dev-loop.ts`), not in the
kernel, so an agent cannot use it today. Proposal: move the pack/verify/
install/update cycle into a kernel operation `AppDevPush` (input: the
packed archive bytes or a local path the kernel may read), keep the file
watching on the client. The CLI's `/app dev` and an agent tool
`app_dev_push` then share one path. In a Room, an agent can: build, push,
open the DEV view, and exercise its own declared actions (§4) to test the
App, with the user watching the same Tab. This is the "agents build Apps
for you" loop and costs one M-size PR.

**Consequences.** Two installations per App ID per owner is a schema change
(`app_installations` gets `channel`), a protocol bump and a Library rule
(dev channel never leaves the kernel). The dev identity is a
product-generated key inside the kernel's key store, like relay and pairing
identities; it must never be enrolled as a publisher.

**Open points for you.**
1. Dev copy as a `channel` of the same App ID (recommend) or a separate
   App ID?
2. Dev builds signed by a kernel-generated dev identity with no prompt
   (recommend), or always the real publisher key as today?
3. Agents in the dev Room see the dev copy automatically (recommend) or
   only when the developer binds them?
4. Resolve the README/code mismatch on approvals: *recommend* the README
   rule (no prompt when capabilities are unchanged) for both channels.

---

## 4. UI-declared agent actions

This is the differentiator. Your words: declare in the SDK that "this is an
action an agent could execute", constrain it to the UI "the same way a user
would interact with the app", and let Chariox translate it into whatever is
most efficient for agents. Then the backend language stops mattering for
agents, which is exactly what WASM makes true for the runtime.

### 4.1 The SDK API in the view

Two forms, both in the view bundle, both framework-agnostic:

```js
// 1. A wrapper around the function the UI already calls.
const addTodo = chariox.action({
  name: "add_todo",
  description: "Add a todo to the current list",
  params: { text: { type: "string", minLength: 1 }, due: { type: "string", format: "date", optional: true } },
  effect: "write",            // "read" | "write" | "approval"
  returns: { id: "string" },
}, async ({ text, due }) => {
  const todo = await chariox.call("todos.add", { text, due });  // same path as the button
  renderList();
  return { id: todo.id };
});
button.onclick = () => addTodo({ text: input.value });  // the user path uses the wrapped function

// 2. Markup for the simple cases: Chariox derives name, params and effect
//    from the form, and runs it by submitting the form.
<form data-chariox-action="rename_list" data-chariox-effect="write"> … </form>

// What the agent reads: a compact state the view publishes.
chariox.view.state(() => ({ list: current.name, todos: current.todos.map(t => ({ id: t.id, text: t.text, done: t.done })) }));
```

The wrapper gives Chariox a typed signature (JSON schema), a description
and an **effect class**: `read` runs without asking anyone; `write`
changes data and runs without a prompt (UX-first, agents are benign);
`approval` routes through the existing kernel-owned interaction shown in
every client (runtime rule 5), the same interaction Vault and sudo use.

### 4.2 Execution: the same path a user triggers

An action runs **inside the view**, in the page's JavaScript, via the
existing CDP binding in reverse (the kernel calls into the page the way it
already delivers `panel` answers). The handler then calls
`window.chariox.call(tool)` exactly as a click does, with
`Actor::Agent` attached (`app_view_runtime.rs:900-930` already does this
for driven views). So:

- the backend never learns a new concept; it sees tool calls with an actor;
- the user sees the change live in the same Tab, because the view that
  rendered it is the one that executed it;
- validation, deadlines, receipts and admission are the ones that exist.

### 4.3 Where the view runs when no human has the App open

The view host already exists: the kernel browser Tab. When an agent needs
an App whose view is not open, the kernel opens the Tab **without
projecting it** to any client (a headless Tab). Engine: the same Chromium.
Cost: one renderer per headless Tab, in the order of 30–80 MiB and a
~300 ms open on a warm browser (to be measured on the real kernel browser;
no number exists today). Rules: one headless view per installation per
Room; it idles out with the App's worker (10 minutes); if a human opens
the App, the headless Tab *becomes* their Tab, so the agent's work is never
done in a copy they cannot see.

Alternative rejected: running the view's action layer in a DOM-less JS
engine inside the kernel. It would be cheaper but would break the one
promise that matters (same UI code path), and would make
`document`-touching handlers fail only when nobody is watching.

### 4.4 What agents get: typed tools, a state snapshot, events, batching

| Surface | Form | Why it beats the alternatives |
|---|---|---|
| Actions | typed MCP tools on the runtime MCP, named `app_<local>_<action>`, schema from the declaration | same path as backend tools today; the model gets a signature, not a screenshot |
| State | one tool `app_<local>_state` returning the published snapshot (JSON, capped at 64 KiB; the view may expose `select` paths for large Apps) | a todo list is ~200 tokens as a snapshot, ~1.5k as a screenshot, 10–50k as DOM |
| Events | the view's `chariox.view.changed(delta)` becomes a live event the agent's turn can wait on, delivered as an MCP notification or appended to the next tool result | the agent does not poll screenshots to learn the state changed |
| Batching | `app_<local>_batch([{action, params}, …])` executes a sequence in one round trip inside the view, stopping at the first error | fewer turns for multi-step edits (fill a sheet, reorder a list) |
| Fallback | if an App declares no actions, agents keep today's path: backend tools and browser driving of the Tab | nothing regresses |

The kernel also publishes the **action manifest** at install (names,
schemas, effects) into the App catalog, so `tools/list` and the Library can
show what agents can do with an App before it is open.

### 4.5 Concurrency: many agents and a user on one App instance

A view is a single JavaScript thread, so actions serialize naturally; the
kernel keeps a bounded per-view queue (same policy as the broker queue in
§2) with the call deadline. The user keeps clicking in the same Tab; the
UI shows agent-made changes as if a collaborator made them, which is the
intended experience. Conflicts are the App's business through its
backend transactions (version checks in `state.transact`), as today.
Several Rooms get several views of one installation; the backend instance
is one per App, shared, with the 64-call concurrency measured in r2.

### 4.6 Discovery review in dev mode

`/app actions` lists the declared actions with schemas and effect classes;
the DEV view shows an **"as the agent sees it"** panel with the current
snapshot and the action list; dev mode warns about interactive controls
(buttons, forms) with no declared action, so coverage is visible. Agents in
the dev Room exercise the actions on the dev copy (§3.5).

### 4.7 Security, UX-first

Guard one thing only: a Vault-filled field in the view is masked in the
snapshot and in any frame sent to the model, and action results never echo
Vault values. Everything else is open: `read` and `write` actions run with
no prompt, `approval` uses the existing interaction once (or zero times
under `/sudo` or an existing grant). No capability prompt is added for
declaring actions.

**Consequences.** New SDK surface in the view bundle (JS only; the backend
language is irrelevant, as you wanted), a kernel action bridge over the
existing CDP binding, a headless Tab mode in the kernel browser, catalog
and runtime-MCP additions, and a protocol bump. The agent-side win does not
depend on WASM at all; the two can land in parallel.

**Open points for you.**
1. Headless Tab in the kernel browser as the view host when nobody has
   the App open (recommend), or actions only while a human has it open?
2. Effect classes `read / write / approval` with `write` unprompted
   (recommend, UX-first)?
3. Should the action manifest be part of the signed package (recommend:
   yes, so the Library can show it and the kernel can diff it on update)?

---

## 5. Why Chariox Apps beat "web app + MCP"

The comparison that matters: the same product built as a web app with a
hand-written MCP server (what every SaaS is doing now) versus a Chariox
App with UI-declared actions.

### 5.1 Agent efficiency: checkable claims

| Claim | Web app + MCP | Chariox App | How to check |
|---|---|---|---|
| One surface, not two | the MCP server is a second implementation of the product's actions; it drifts from the UI and covers a subset | actions *are* the UI's functions; coverage is visible in dev mode (§4.6) | count actions vs UI controls in the benchmark Apps |
| The user sees what the agent does | MCP calls change data behind the UI; the user refreshes to find out | the action runs in the view the user is looking at | recording in the benchmark |
| Tokens per step | MCP: a tool call (~100–300 tokens) but the model must separately learn state through more calls; Computer Use: one screenshot ≈ 1.5k tokens per step | one typed call plus one ~200-token snapshot; batching cuts steps | token counts per task |
| Latency per step | network RTT to the SaaS and its MCP (50–300 ms) plus auth | local view call: the kernel's tool path is tens of µs, the view handler a few ms | timings per step |
| Failure modes | schema drift between UI and MCP; stale OAuth; rate limits | typed params checked by the kernel; same state as the UI | failure rate per task |
| Approval | per-server allow-lists in each agent client | one kernel-owned interaction, effect classes from the developer | n/a (design) |
| Local-first | data lives at the vendor | data lives in the kernel, every OS, offline | n/a |

### 5.2 Developer efficiency: checkable claims

| Claim | Web app + MCP | Chariox App |
|---|---|---|
| Code to write for agent access | an MCP server: transport, auth, tool schemas, hosting, versioning | one wrapper per action in the view (§4.1), or a `data-` attribute |
| Infrastructure | a server, a database, auth, TLS, uptime | none: the kernel runs the backend and stores state |
| Portability | per-OS packaging if there is a desktop part | one `.cxapp` for every OS and CPU |
| Backend language | whatever the server runs | Rust, JS/TS, C (and C++, Python later) with one typed contract |
| Dev loop | deploy to test agents against it | `/app dev`: ~1.5–4 s per save, agents in the Room test it live |
| Distribution and trust | app store + MCP registry, separate | signed package, Library, kernel-verified imports |

### 5.3 Benchmark design (to run before any public claim)

- **Apps:** Todo (small) and a Sheet (Univer, §7b). Both exist as a Chariox
  App with declared actions, and as a plain web app plus a small MCP server
  (same backend code, the MCP handwritten by the same developer, time
  recorded).
- **Arms:** (A) Chariox App actions, (B) web app + MCP server, (C) web app
  through Computer Use (screenshots), (D) Chariox App through browser
  driving only (no declared actions), as the control for the UI itself.
- **Tasks (10, scripted, each with a checker):** add 5 todos with due
  dates; complete all overdue; rename a list; import a CSV into a sheet;
  fill a column with a formula; sort and export; recover from a validation
  error; and so on.
- **Agents:** Codex `gpt-6-luna` low and Claude `claude-haiku-5-5`, 30 runs
  per task per arm.
- **Measures:** tokens in and out, wall-clock per task, steps (tool calls
  or screenshots), failure rate (checker fails), and developer minutes to
  build each arm.
- **Pass:** (A) uses fewer tokens and steps than (B) and (C) on every task,
  fails less, and took fewer developer minutes than (B).

**Consequences.** The claims above are not proven today; the benchmark is
an S-size PR once §4 exists, and should gate the "Apps beat MCP" message.

**Open points for you.** Is the comparison you want to win (B) (SaaS + MCP),
(C) (Computer Use), or both? *Recommend: both, with (B) as the headline.*

---

## 6. Languages

| Language | Toolchain | Async world | Measured | Launch? |
|---|---|---|---|---|
| Rust | wit-bindgen 0.63, `wasm32-wasip2` | yes, full cancel | r2: all cells | **yes** |
| JS/TS | componentize-qjs (pinned git), Rolldown bundle | yes; cancel does not stop the pending import | r2: all cells | **yes** |
| **C** | wasi-sdk 33 (clang 22), wit-bindgen 0.63 `c` | **yes, full cancel** | **this round: all cells** | **yes, with a thin SDK (see below)** |
| C++ | same, through the C SDK; C++20 coroutines map onto the callback ABI | yes (same ABI) | not run | after C |
| Python | componentize-py 0.25.1 | yes; cannot cancel (traps on an abandoned future read) | r2 | preview |
| Go | TinyGo `wasip2` + wit-bindgen-go | no async yet | not run | state only |

### 6.1 C: the measured answer to "from the get go?"

**Yes, C works today on the async world, with no SDK and no C-specific
worker change.** `spikes/wasm/apps/c-app/app.c` (≈330 lines) implements
the same ten tools as the Rust App against `wit/app.wit`, compiled with
`wit-bindgen c` (callback ABI) and `wasm32-wasip2-clang -mexec-model=reactor`,
and ran every r2 cell on the unchanged async worker under bubblewrap:

| Cell (C, bubblewrap unless noted) | Result | Rust / QuickJS for comparison |
|---|---|---|
| build per save | **0.22 s** (bindgen + clang) | Rust: seconds; QuickJS 0.26 s |
| component / precompiled | 302 KB / 222 KB in 16 ms | 190 KiB / 595 KiB; 1.5 MB / 4.6 MB |
| cold start to ready | 7.2–8.2 ms (1.7–2.9 ms without bubblewrap) | 5.5 / 6.1 ms |
| RSS after ready / after work | 13 / 15 MiB | 14 / 25; 17 / 33 |
| tool call p50 / p95 | 0.03 / 0.04 ms | 0.03 / 0.04; 0.04 / 0.06 |
| async imports (`state.get`, `transact`, `http.send`) | work, results land in the task's struct | same |
| 16 callers, one broker wait | 16/16 ok, no lost update, one connection, 52.9 ms | 53.0 / 53.1 |
| 64 callers, WASI 0.3 timer | **64/64 ok**, 52.4 ms | 64/64 |
| 64 callers, broker wait | 16 ok, 48 `BUSY` (kernel cap, as every runtime) | same |
| 50 MiB streamed body | 231 MiB/s, 4 chunks read-ahead, +1 MiB RSS | 265 / 270 |
| cancel 2 of 10 calls | **import and stream released in 12.5 ms**, 8 others ok, next call 0.1 ms | Rust 12 ms; QuickJS 5.3 s |
| runaway loop | stopped at 1.0 s, culprit named, 8 others `APP_INSTANCE_RESTARTED` | same |
| 256 MiB with cap 128 | `malloc` returns NULL: the App reports `APP_MEMORY_LIMIT` itself, **8 others ok, state kept** | Rust traps (state lost); QuickJS same as C |
| head-of-line | a 1M-op C loop holds echo 0.2 ms (the C loop is far cheaper than Rust's HashMap workload; not comparable) | 188 ms |

What the test shows about the toolchain:

- `wit-bindgen c` generates the full async ABI: subtasks, waitable sets,
  stream and future reads, `context.set/get` for per-task state. Nothing
  was missing for our world.
- A C App is a **hand-written state machine**: every `await` is a return
  to the host plus a callback. That is how `app.c` is written; it is fine
  for a spike and tedious for a developer. The thin C SDK must hide it:
  a `chariox_task` struct, `CHARIOX_AWAIT(call, next_step)` macros (as in
  the spike), helpers that build requests and answers, and a small JSON
  library (there is none in wasi-libc; cJSON is MIT). C++ gets it for free
  with coroutines (`co_await` on an import), which is why C++ is "through
  the C SDK".
- One trap to encode in the SDK: synchronous builtins (`cancel-read`,
  `drop`) trap on a waitable that is still joined to a set; remove it
  first. The first run hit exactly this.
- wasi-libc still imports WASI 0.2 (`wasi:cli`, `wasi:io`, `wasi:clocks`);
  the worker links 0.2 as a compatibility layer already (DESIGN.md §2).
- A blocking style (`chariox_await()` that just waits) would need the
  stackful async lift, which wasmtime supports and `wit-bindgen c` does
  not generate today. Not needed for launch; it would make C look like
  Rust.

So: C at launch is a product decision about the SDK, not a feasibility
question. The SDK is an M-size PR (§8, L2).

### 6.2 The others

- **C++:** header-only layer over the C SDK; coroutines replace the state
  machine. Exceptions across the component boundary must be caught at the
  export (no unwinding into the host). S after the C SDK.
- **Python:** works and is slow to start (19 MB component, 8 ms cold,
  0.7 s precompile) and cannot cancel. Preview after launch; the trigger is
  componentize-py being able to abandon a future read.
- **Go:** wait for async support in the Go toolchain; say so in the docs.

**Consequences.** The world is the contract; every language is an SDK on
top. Launch set: Rust, JS/TS, C. C++ follows in weeks, Python as a preview.

**Open points for you.**
1. C at launch with the state-machine SDK (recommend), or wait for a
   blocking-style SDK?
2. Ship C++ at launch too (header-only, S after C) or right after?
   *Recommend: right after; do not widen the launch.*

---

## 7. Wrapped existing binaries

Your ask: existing programs hooked as Chariox Apps even with no MCP,
driven by agents through the UI in Chariox, reachable from a remote client.

| Kind of program | How it runs | How clients see it | How agents act | Feasibility |
|---|---|---|---|---|
| GUI program (Linux) | a separate App kind `kind: "native"`, per-OS binary, OS sandbox (same bwrap/cgroup launcher as the worker), on a **virtual display** (Xvfb/Wayland headless) | streamed with the Browser display transport that replaces Selkies video (multidomain) | accessibility tree (AT-SPI) where the toolkit exposes one; Computer Use (screenshot + click) otherwise | good: we already run Chromium this way; the display transport is being built for the browser anyway |
| GUI program (macOS) | native binary in the OS sandbox; no virtual display on macOS, so **per-window capture** of a real window (ScreenCaptureKit) | same transport | macOS AX API (rich on native apps) or Computer Use | possible, more work; headless use (nobody logged in) does not exist on macOS |
| GUI program (Windows) | not today | — | — | later |
| CLI program | a Chariox terminal pane with the binary in the OS sandbox | the existing pane/terminal transport | the agent's terminal tools, as with any CLI | good: mostly packaging and sandbox profile |
| Existing web app | a Tab of the kernel browser pointing at it | the Browser mirror (DOM mirroring, region video) | browser driving, as today | good: already the product |

Limits that stay true for wrapped programs: no portability (one binary per
OS), no per-call limits (the sandbox limits the process, not a call), no
typed actions unless the program exposes them through accessibility, and
trust is only the OS sandbox plus the publisher signature on the wrapper.
Agents interact through accessibility or pixels, which is slower and
less reliable than §4's declared actions; the benchmark arm (C) measures
exactly that gap.

Relation to native Chariox Apps: the wrapped kind is the on-ramp
("my program in Chariox today"); the WASM kind is the destination
("portable, limited, agent-native"). The manifest, signing, install,
generations, approvals and the view Tab are shared; only `runtime` differs.

**Consequences.** A real but separate L-size track: sandbox profiles per OS
for arbitrary binaries, a virtual display per App on Linux, window capture
on macOS, and an accessibility bridge for agents. None of it blocks §2–§4.

**Open points for you.**
1. Scope for v2: Linux GUI + CLI wrapping first, macOS window capture
   second, Windows not planned? *Recommend: yes, and only after §4 lands.*
2. Should wrapped GUI Apps be able to **declare actions** too, through
   accessibility scripts (e.g. "click the Save menu")? *Recommend: later;
   see how far AX gets us first.*

---

## 7b. Adapting existing open-source projects

Two filters: (a) is the UI already web? (b) does the backend compile to a
WASI component (no threads, no raw sockets, no native GUI), or can the SDK
brokers replace it? Licences verified 2026-10-22 from each repo's LICENSE
or licence page; releases from GitHub.

| Project | Licence | Flag | (a) web UI | (b) backend | State | Fit |
|---|---|---|---|---|---|---|
| Excalidraw | MIT | — | yes (React component) | none needed | v0.18.1 (04-2026), very active | **Good** |
| Univer (sheets/docs/slides) | Apache-2.0 core; Pro is commercial | xlsx/docx import-export, charts, pivot, print, collaboration are **Pro-only** | yes (canvas) | none needed; add our own xlsx I/O | v1.2.0 (10-22-2026), very active | **Good** |
| Typst | Apache-2.0 (typst and typst.ts) | — | partly (renderer exists, editor to assemble) | **yes**: Rust → `wasm32-wasip2`, fonts/packages via HTTP + file brokers | v0.15.1 (07-2026) | **Good** (best real backend component) |
| PDF.js | Apache-2.0 | — | yes | none needed | v6.4 (10-2026) | Good |
| pdf-lib | MIT | upstream dormant since 2021; use the maintained `cantoo-scribe` fork (MIT, 09-2026) | library | pure JS → QuickJS backend | fork active | Good as a library |
| SQLite (sqlite-wasm) | public domain / Apache wrapper | — | n/a | C builds with wasi-sdk (rusqlite bundled works on wasip2) | 3.54 (10-2026) | Good as the storage of any App |
| Monaco / CodeMirror 6 | MIT / MIT | CodeMirror's GitHub mirror archived 04-2026, canonical repo moved; npm current | yes | none | current | Good building blocks |
| reveal.js | MIT | — | yes | none | 6.0.2 (09-2026) | Good |
| Vega-Lite | BSD-3 | — | yes | none | v6.5 (10-2026) | Good; JSON spec is agent-friendly by itself |
| Actual Budget | MIT | — | yes (React) | runs in-browser on sql.js WASM; sync-server optional → state/file broker; bank sync via HTTP broker | v26.10 (10-2026) | Possible (big) |
| draw.io | Apache-2.0 source; icon/stencil sets under separate non-OSI terms (Atlassian carve-out); repo now called a "test version", paid production build | **asset terms, not the code** (the "editor moved off Apache in 2024" premise did not verify) | yes | none needed (export server optional) | v32.5 (10-2026), closed to PRs | Possible; Excalidraw is cleaner |
| DuckDB (duckdb-wasm) | MIT | — | n/a | Emscripten only (exceptions, pthreads): runs in the **view's** Web Worker, not as a component (~30 MB) | v1.33 (12-2025) | In-view only |
| Squoosh codecs | Apache-2.0 (codecs own BSD/MIT) | project dormant since 08-2024 | yes | Emscripten C++/Rust codecs; Rust ones trivial to rebuild for wasip2 | dormant | Possible (reuse codecs) |
| MuPDF / mupdf.js | **AGPL-3.0** or commercial | **AGPL** covers the WASM binary; network-use copyleft | library | Emscripten; C core could rebuild with wasi-sdk | mupdf.js 0.3 (08-2024) | Avoid; PDF.js + pdf-lib cover it |
| pandoc | **GPL-2.0+** (pandoc-wasm wrapper MIT) | **GPL**: bundling pandoc.wasm makes the App GPL | CLI | yes, GHC wasm backend builds a WASI p1 module (no Lua filters) | 3.12 (10-2026) | Wrapped binary / sidecar only |
| LibreOffice WASM (ZetaJS) | MPL-2.0 (+LGPL parts); zetajs MIT | weak copyleft | **no**: Qt-for-WASM canvas | **no**: Emscripten + pthreads + SharedArrayBuffer (needs COOP/COEP), ~32 MB wasm + data, beta | zetajs 1.2 (06-2025) | No → **wrapped binary (§7)** |
| Collabora Online | MPL-2.0 | weak copyleft | yes (JS client) | **no**: native C++ server + LibreOfficeKit processes | active | No → wrapped |
| OnlyOffice Docs | **AGPL-3.0** | **AGPL** | yes (canvas) | **no**: Node services + x2t C++ converter + RabbitMQ/PostgreSQL | v10 (10-2026) | No → wrapped, and AGPL |

### 7b.1 The two to build: Excalidraw first, Univer sheets second

**Excalidraw (diagrams).** Views: the Excalidraw React component as the
single UI entry, state kept by the App. Backend component (QuickJS or
Rust): scene store (elements JSON in `state`, images through `files`),
library/palette storage, PNG/SVG export. Declared actions: `add_shape`,
`add_text`, `connect(a, b, label)`, `move/group`, `set_style`,
`load/save scene`, `export(png|svg)`, and `describe_scene` (the snapshot:
elements with ids, text, positions and connections). What the agent can
then do that it cannot through Excalidraw + MCP today: there is no
Excalidraw MCP; with Computer Use it must draw by pixels. Here it reads
the structured scene and edits it by id while the user watches the same
canvas, offline, in the user's kernel.

**Univer (sheets).** Views: the Univer sheet workbench. Backend component
(Rust): workbook persistence (snapshot JSON via `state`/`files`), CSV and
xlsx import/export (Univer's own are Pro-only; use a Rust xlsx crate),
formula recalculation stays in Univer's engine in the view. Declared
actions: `get/set_range`, `insert/delete rows|columns`, `set_formula`,
`add_sheet`, `sort/filter`, `import_csv`, `export(xlsx|csv)`; snapshot =
the active sheet's used range (capped) plus sheet names. Beyond the
original + MCP: a spreadsheet the agent edits cell by cell with typed
calls and immediate visual feedback, with the data never leaving the
machine; Univer has a Facade API but no MCP and no sandboxed local runtime.

Typst is the best "real backend" showcase (a Rust compiler as a component
with brokered fonts and packages) and the third candidate once the first
two exist.

**Consequences.** Two M-size reference Apps that also serve as the
benchmark Apps (§5.3). Licence rule for first-party Apps: permissive only
(MIT/BSD/Apache/MPL as a dependency); no GPL/AGPL code in a Chariox App we
ship; copyleft programs go through the wrapped kind if at all.

**Open points for you.**
1. Excalidraw first, Univer second (recommend)?
2. The licence rule above for first-party Apps (recommend: yes).
3. Univer's Pro-only features: build our own xlsx I/O (recommend) or
   accept csv-only at first?

---

## 8. Revised migration plan and your decisions

Sizes: S ≈ 1–2 days, M ≈ 3–5 days, L ≈ 1–2 weeks, one lane each. Apps are
paused; this is the order to resume in. Tracks run in parallel; the WASM
runtime (R) and the agent-side UI work (A) do not depend on each other.

| # | PR | Size | Depends on |
|---|---|---|---|
| R1 | Async WIT world 1.0 + import allowlist (`packages/app-wit`) | S | — |
| R2 | Kernel: queue broker calls per App instead of `BUSY`, cap 64; worker-side cancel; protocol bump + drill | M | — |
| R3 | Async wasm worker (from `worker-async/`): streams, cancel, CPU slices, culprit, OOM recycling, `precompile`/`check` | L | R1 |
| R4 | Kernel install + launch for `engine: "wasm"`, AOT cache in the release store | L | R3 |
| R5 | Sandbox profiles for the worker (seccomp, Seatbelt, executable memory) | M | R3 |
| R6 | WASI virtualization: `wasi:http` onto the broker, preopens | M | R3 |
| L1 | JS SDK on QuickJS + `app pack` bundling + pinned componentize-qjs; Rust SDK crate | L | R1 |
| L2 | **C SDK** (task helpers, await macros, JSON, request builders) + template; C++ header after | M (+S) | R1 |
| D1 | Dev channel: `channel` on installations, dev identity, `/app register`, kernel reload of dev views, measured save→serve times | M | — |
| D2 | `AppDevPush` kernel operation + agent tool; `/app dev` on top | M | D1 |
| A1 | View SDK: `chariox.action`, `data-chariox-action`, `view.state`, `view.changed`; action manifest in the package | M | — |
| A2 | Kernel action bridge: run actions in the Tab via the CDP binding with `Actor::Agent`; per-view queue; `app_<local>_<action>`, `_state`, `_batch` on the runtime MCP; protocol bump | L | A1 |
| A3 | Headless Tab mode in the kernel browser + cost measurement + idle rules | M | A2 |
| A4 | Dev-mode discovery: `/app actions`, "as the agent sees it" panel, undeclared-control warnings | S | A1, D1 |
| P1 | Port first-party Apps to wasm, delete the Node engine, real-path drills | M + S | R4, L1 |
| X1 | Excalidraw App; X2 Univer sheets App (also the benchmark Apps) | M each | A2, L1 |
| B1 | Benchmark (§5.3) and the public claim | S | X1 |
| W1 | Wrapped-binary kind: Linux GUI on a virtual display + CLI panes; W2 macOS window capture | L, L | later |

Critical path to "Apps v2 usable": R1 → R3 → R4 → P1 (≈ 6 weeks) in
parallel with A1 → A2 → A3 (≈ 4 weeks) and D1 (1 week). C and the
reference Apps follow inside the same window.

### Your decisions (each with my recommendation)

| # | Decision | Recommendation |
|---|---|---|
| 1 | Broker cap: queue instead of `BUSY`, 64 in flight per App | yes |
| 2 | Debugger gap at launch (logs + stack traces, no step debugger) | accept |
| 3 | Dev copy = `channel: "dev"` of the same App ID, signed by a kernel-generated dev identity without a prompt | yes to both |
| 4 | Agents in the dev Room see the dev copy automatically | yes |
| 5 | Approvals: no prompt when capabilities are unchanged (README rule), for both channels | yes |
| 6 | Headless Tab in the kernel browser hosts the view when nobody has the App open | yes, measure cost in A3 |
| 7 | Effect classes `read / write / approval`; `write` runs unprompted | yes (UX-first) |
| 8 | Action manifest inside the signed package | yes |
| 9 | Benchmark target: SaaS + MCP (headline) and Computer Use | both |
| 10 | C at launch with the state-machine SDK; C++ right after | yes |
| 11 | Python as a preview after launch; Go wait | yes |
| 12 | Wrapped binaries: Linux GUI + CLI first, macOS second, after A-track lands | yes |
| 13 | Reference Apps: Excalidraw first, Univer sheets second; permissive-licence rule for first-party Apps | yes |
| 14 | Resume order: R-track and A-track in parallel, D1 first week | yes |

What is **not** asked of you: the detailed WIT, the SDK function names,
the queue sizes and the sandbox profiles are engineering choices inside
the PRs above.
