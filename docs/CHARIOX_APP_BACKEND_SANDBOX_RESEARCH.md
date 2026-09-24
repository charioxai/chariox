# Chariox App backend runtime and sandbox research

Status update, 2026-09-05: this research predates the full plan review. The [platform review](CHARIOX_APPS_PLATFORM_REVIEW.md) corrects its broad Node compatibility, resource-limit and storage claims. The updated Apps implementation plan governs the agreed capability subset, deferred filesystem transactions, human validation and two release gates.

Status: design recommendation based on primary sources and the current Chariox architecture.

## Decision

Chariox can guarantee that every valid third-party App backend runs in a browser-grade sandbox on each supported kernel machine. This needs to be a product invariant, not a best-effort runtime option.

The recommended design is:

1. Ship one pinned Node runtime inside a dedicated `chariox-app-worker` executable. Do not invoke a developer's system Node.
2. Run one App worker process per active installation under the kernel-owned App supervisor. Treat App code as malicious from its first instruction.
3. Integrate Chromium's platform sandbox components or equivalent policies into the supervisor and worker launch path. The sandbox must start before Node and before App code loads.
4. Permit normal `node:fs` access to the immutable package and the installation's private data and temporary directories. Deny every other host path at the OS boundary.
5. Deny raw network sockets. Provide a fetch-compatible kernel broker for declared service destinations and opaque Chariox connection handles.
6. Define the portable v1 backend contract as JavaScript plus pure-JavaScript npm packages. Native addons, FFI, package install scripts, and subprocesses are outside that portable contract.

This is how Chromium's model applies to Chariox: one privileged broker owns policy and resources, while separate restricted processes run untrusted code. Chromium states that it assumes sandboxed code is malicious and relies on OS security for its guarantees ([Chromium sandbox design](https://chromium.googlesource.com/chromium/src/+/main/docs/design/sandbox.md)).

Node is the better v1 runtime once the OS sandbox is mandatory. Deno's permissions remain useful defense in depth, but they do not remove the need for an OS sandbox. Deno itself recommends OS sandboxing for untrusted code. Native libraries and subprocesses can bypass Deno's permission sandbox ([Deno security and permissions](https://docs.deno.com/runtime/fundamentals/security/)). Node gives developers exact Node and npm behavior instead of a compatibility layer.

## The system being isolated

A Chariox App and the service it connects to have three product components:

| Component | Execution owner | Role |
| --- | --- | --- |
| App view, the frontend | The Room's kernel-managed Environment browser | Human UI |
| App backend | The kernel's App supervisor on the kernel machine | Developer JavaScript, agent tools, events, local App data, and calls to the service |
| Service backend | The third-party service | Existing API, authentication domain, and database |

Use two additional terms only for Chariox internals:

| Internal term | Meaning |
| --- | --- |
| App supervisor | The trusted kernel component that owns App installation policy, starts workers, applies containment, enforces resource limits, and routes bounded IPC. |
| App worker | One sandboxed child process that embeds pinned Node and runs one installed App backend. |

Chromium documentation calls the equivalent security roles the broker and target. Those words describe the sandbox pattern; Chariox product and implementation documentation uses App supervisor and App worker.

The terminal is a client of the kernel. It does not own any of these runtime components. The Chariox architecture already says that one Room owns one shared Environment, while browser, controller, streamer, and desktop processes only execute it ([architecture, section 4.2.4](ARCHITECTURE.md#424-shared-room-environment-authority); [ADR 0001](adr/0001-one-shared-environment-per-room.md)).

The App backend sandbox is separate from the browser sandbox around the App frontend. A compromised frontend must pass through the kernel-authorized bridge. A compromised backend must still be unable to read provider credentials, Chariox state, user files, another App's data, or arbitrary network services.

## What Chromium does

Chromium does not use one portable container implementation. It uses the same broker and target architecture with a different OS mechanism on each platform:

| Platform | Chromium mechanism | Relevant behavior for Chariox |
| --- | --- | --- |
| Linux | User or PID/network namespaces for the first isolation layer, then seccomp-BPF to reduce kernel calls | A launcher creates the isolation boundary and the target installs a process-specific syscall policy. Chromium also brokers selected calls over IPC. ([Linux sandboxing](https://chromium.googlesource.com/chromium/src/+/main/docs/linux/sandboxing.md)) |
| macOS | Seatbelt `sandbox(7)` profiles with default deny | Chromium compiles a policy, passes it to its helper, and applies it before loading the main framework. Policies can grant named paths and broker controlled operations. ([Chromium macOS sandbox](https://chromium.googlesource.com/chromium/src/+/main/sandbox/mac/); [Seatbelt V2 design](https://chromium.googlesource.com/chromium/src/sandbox/+/HEAD/mac/seatbelt_sandbox_design.md)) |
| Windows | Restricted tokens, Job objects, alternate desktops, integrity levels, process mitigations, and LPAC or AppContainer | The broker creates the target and can grant path-specific file access. A process-specific SID can protect one sandbox's files, and an AppContainer without the Internet capability has no network access. ([Chromium sandbox design](https://chromium.googlesource.com/chromium/src/+/main/docs/design/sandbox.md)) |

Chromium maintains these as platform-specific libraries under `//sandbox`, with concrete process policies above them ([Chromium sandbox library](https://chromium.googlesource.com/chromium/src/sandbox/)). The code is BSD licensed ([Chromium license](https://chromium.googlesource.com/chromium/src/+/main/LICENSE)).

This explains why Chromium works on all three desktop platforms. It ships, tests, and updates all three implementations as part of the browser. Chariox can do the same for one narrower App process type.

### Capabilities blocked from Chromium web content

Chromium makes a sharp distinction between trusted browser services and an untrusted renderer. Page JavaScript cannot directly invoke the operating-system functionality listed below.

| Capability | Chromium renderer and web-content behavior | Chariox App backend equivalent |
| --- | --- | --- |
| Native add-ons and FFI | Web content has no Node native-module loader or general FFI API. WebAssembly receives only the imports that the browser exposes and has no ambient system access. | Phase 1 allows portable WebAssembly without WASI. Native Node add-ons need reviewed and signed platform packages before they can be enabled inside the OS sandbox. |
| Child processes and host executables | Web content has no process-spawn API. Chromium's renderer policy prevents process creation and brokers the few permitted operations over IPC. | Phase 1 denies `child_process`. A later declared helper must be packaged, reviewed, and launched by the App supervisor inside the same installation sandbox. |
| Raw TCP and UDP | Web content gets Fetch, XMLHttpRequest, WebSocket, and other policy-controlled web APIs. It does not receive raw socket handles. Chromium keeps trusted network interfaces away from renderers and implements networking in a browser-controlled service ([Chromium Network Service](https://chromium.googlesource.com/chromium/src/+/main/services/network/README.md)). | Phase 1 supplies brokered Fetch. Direct database drivers, SSH, SMTP, arbitrary listeners, and custom socket protocols need a specific Chariox broker. |
| Arbitrary files | Renderers cannot read or write arbitrary host paths. Browser storage, downloads, uploads, and user-approved file access are mediated by the browser. | The App worker receives broader local storage than a renderer, but only inside its package, data, and temporary roots. External files use a revocable kernel grant. |
| Runtime package installation | A page does not run npm install hooks or mutate the browser executable. It loads web resources under origin, CSP, cache, and storage rules. | Chariox resolves and builds dependencies before packing. App installation activates immutable bytes and never runs npm lifecycle scripts. |
| Native application integration | Chrome extensions use a separately installed native-messaging host in another process, connected through a bounded message protocol ([Chrome native messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging)). | A future Chariox native helper follows the same separation: declared executable, separate process, bounded IPC, explicit capability, and OS containment. |

Chromium's sandbox FAQ says a renderer can freely use CPU and memory but cannot write to disk or create its own windows; a privileged browser process performs approved work over explicit channels ([Chromium sandbox FAQ](https://chromium.googlesource.com/chromium/src/+/main/docs/design/sandbox_faq.md)). Chariox follows the same rule while deliberately granting its App backend a private filesystem and a small set of background capabilities.

### Chromium's sandbox is not a command wrapper

Chariox cannot point the current Chromium instance at a stock `node` or `deno` process and inherit renderer isolation.

On Windows, Chromium's sandbox is a static library linked into both the broker and target executables. The target contains the sandbox IPC client, policy client, and API interceptions. On macOS, Chromium's helper applies the Seatbelt profile before it loads the main Chromium framework. On Linux, namespace setup, zygote behavior, and seccomp policy are integrated into the process model. These facts make reuse possible, but only through a Chariox-owned target executable and policy integration.

Electron is useful counter-evidence. Its sandboxed renderers have no Node environment; enabling Node integration disables the renderer sandbox. Electron's full-Node utility process uses a `node.mojom.NodeService` declared with `kNoSandbox` ([Electron process sandboxing](https://www.electronjs.org/docs/latest/tutorial/sandbox); [Electron Node service declaration](https://github.com/electron/electron/blob/main/shell/services/node/public/mojom/node_service.mojom)). Electron therefore does not provide a ready-made sandboxed Node backend for Chariox.

Chromium already defines `kServiceWithJit` for computation services that need V8 or WebAssembly JIT without general OS access ([Chromium sandbox types](https://chromium.googlesource.com/chromium/src/+/HEAD/sandbox/policy/mojom/sandbox.mojom)). That is the closest policy model for an App worker, but Chariox needs its own file and broker allowances.

## Concrete App worker design

### Process structure

```text
kernel App supervisor
  |
  | inherited authenticated IPC handle only
  |
  +-- chariox-app-worker for installation A
  |     +-- embedded pinned Node
  |     +-- immutable App JavaScript and dependencies
  |     +-- OS sandbox for package/data/tmp
  |
  +-- chariox-app-worker for installation B
        +-- separate sandbox identity and resource limits
```

The kernel starts the target, fixes its policy, passes only the required handles, waits for the sandbox handshake, and then allows Node to initialize and load the App. App code never runs in the kernel process or on a kernel authority loop.

Node provides a C++ embedder API for executing JavaScript in a Node environment from another executable. Node warns that this API may break on semver-major releases, so Chariox must pin and update the worker and Node together ([Node C++ embedder API](https://nodejs.org/api/embedding.html)). That maintenance cost is real, but bounded to one executable and one supported runtime version.

The IPC connection identifies installation ID, release digest, worker generation, and request ID from the supervisor-created channel. The App receives no bearer secret in environment variables or command-line arguments. The kernel validates every SDK call against the active release and recorded grant.

### Files

The OS sandbox exposes:

```text
package   read only   packaged runtime code and node_modules
data      read/write  durable installation data
tmp       read/write  bounded disposable data
```

The App uses ordinary `node:fs`, streams, and libraries within those directories. Chariox sets the working directory and provides `chariox.paths.package`, `chariox.paths.data`, and `chariox.paths.tmp`. Common libraries that accept a data path need no I/O adapter.

The paths do not need to appear as a literal `/` on every OS. Linux mount namespaces can provide that view. Seatbelt on macOS and LPAC ACLs on Windows enforce allowed host paths without changing path syntax. The security guarantee is that OS path resolution cannot escape the allowed directories, including through symlinks, junctions, reparse points, inherited descriptors, or rename races.

Node's permission model should also run in enforce mode as defense in depth, but Node explicitly says that it does not protect against malicious code. It also documents bypasses through existing file descriptors and OS-level process signaling ([Node permission model](https://nodejs.org/api/permissions.html)). Node's experimental virtual filesystem likewise says it is not an isolation boundary ([Node virtual filesystem](https://nodejs.org/api/vfs.html)). Neither replaces the platform sandbox.

Direct filesystem access changes the current plan's storage promise. Chariox can enforce scope, quota, snapshots, and lifecycle. It cannot make arbitrary library writes transactional. The SDK may still provide atomic replace, compare-and-set, and transactions for Apps that need them. Plain `node:fs` follows normal filesystem crash semantics.

### Network and credentials

The target has no general socket capability. All outbound calls use a fetch-compatible SDK implementation over the inherited IPC channel. Chariox can expose it as both `chariox.fetch` and the global `fetch`, so ordinary web API clients that depend on Fetch need little or no change.

The broker enforces declared scheme, host, port, redirect, DNS rebinding, IP-literal, loopback, private-network, and link-local rules. It resolves opaque connection handles and applies credentials without returning them to the App. Raw `node:net`, `node:dgram`, `node:http`, inbound listeners, and Unix sockets other than the inherited Chariox channel fail at the OS boundary. Packages built directly on Node socket APIs need a Chariox transport adapter. This is a deliberate limit of the portable contract.

This broker is closer to browser behavior than granting a process an allowlisted `--allow-net` flag. Chromium renderers ask privileged browser services to perform work; the browser process enforces policy and returns constrained results. It also gives Chariox one place to audit requests and revoke a connection immediately.

### Resource and lifecycle controls

The worker sandbox policy denies child process creation, inspector attachment, FFI, native dynamic libraries outside the signed Chariox runtime, devices, interactive desktop access, kernel and provider IPC endpoints, and access to other processes. The supervisor applies memory, CPU, process, file-descriptor, output, and IPC-frame limits. It terminates only the offending App on a breach or deadline.

Platform implementation follows the Chromium pattern:

- Linux enters user, mount, PID, and network namespaces, maps package/data/tmp, drops privileges, applies cgroup limits, and installs seccomp-BPF before Node starts. The Chariox Linux support floor must require the namespace and seccomp facilities it ships against. If a distribution requires a privileged helper, the Chariox installer must install and verify that helper as Chromium does with its setuid sandbox.
- macOS uses a signed Chariox helper with a parameterized, default-deny Seatbelt profile. It applies the profile before loading embedded Node. The package must use the hardened runtime, be notarized, and carry the JIT entitlement required by V8. Apple documents that notarized software using a JavaScript engine needs the JIT exception ([Apple notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution); [JIT entitlement](https://developer.apple.com/documentation/BundleResources/Entitlements/com.apple.security.cs.allow-jit)).
- On Windows, the App supervisor takes Chromium's broker role and the App worker is its target. LPAC gives each installation a process-specific SID; the installer lays down read-only package ACLs and read/write data ACLs. Chromium documents that LPAC data and binary ACLs must be created by the installer. A Job object supplies process and resource containment.

A passing sandbox self-test is part of installing Chariox support on a kernel machine. There is no unsandboxed fallback and no per-App decision to weaken the policy. Once a machine supports Chariox Apps, every valid App uses the same enforced path. Failure to start that path is a kernel runtime failure, not an invitation to run the App without isolation.

## What counts as a valid portable App

"Runs any App" needs the same kind of contract as "Chromium runs any web app." Chromium runs programs written for the Web platform. It does not run arbitrary local executables with the browser sandbox disabled.

For Chariox v1, a valid portable backend should contain:

- JavaScript entry code and pure-JavaScript npm runtime dependencies
- a complete lockfile and deterministic package graph
- no runtime download or dynamic remote import
- no native `.node` addon, FFI library, executable, or subprocess
- no npm lifecycle script that must run on the user's machine
- service access through Fetch or the Chariox SDK
- local I/O confined to package, data, and temporary paths

Build-time React, Vite, TypeScript, code generation, and similar Node tooling remain unrestricted developer choices. The `.cxapp` contains the resulting UI and backend artifacts.

Library releases should install dependencies and execute any approved build scripts in disposable review infrastructure, then publish immutable package bytes. A local developer build does the same before packing. The installed kernel never runs npm lifecycle scripts while activating an App.

Native addons and subprocesses are a separate compatibility class:

| Feature | Why it is separate | Possible later contract |
| --- | --- | --- |
| Node-API/native addon | It is platform and architecture specific, runs machine code, changes code-signing and library-loading policy, and may need install scripts | Require per-platform artifacts, Chariox review and signature, no runtime compilation, and continued OS sandboxing |
| FFI | It can call arbitrary syscalls and bypass runtime permissions | Keep unsupported unless it becomes the same reviewed native-artifact class |
| Subprocess | It introduces another executable, process tree, and syscall profile | Let the App supervisor launch only a declared packaged executable so it inherits the installation sandbox and limits |

Deno does not avoid this split. Deno states that `--allow-ffi` lets native code issue syscalls directly and that `--allow-run` can escape the permission sandbox. Deno's Node-API support also needs local `node_modules`, `--allow-ffi`, and often explicitly approved lifecycle scripts ([Deno Node and npm compatibility](https://docs.deno.com/runtime/fundamentals/node/)).

## Node compared with Deno for this backend

| Question | Node | Deno | Effect on the decision |
| --- | --- | --- | --- |
| Existing `package.json`, CommonJS, npm behavior | Native behavior | Broad compatibility, with documented resolution and API gaps | Node requires fewer developer changes |
| Pure-JavaScript npm packages | Native behavior | Most work, according to Deno, but not all | Both are viable for selected fixtures |
| Runtime permission system | Explicitly not a malicious-code boundary | Denies I/O by default, but recommends OS isolation for untrusted code | Neither removes the mandatory App worker sandbox |
| Native addons and subprocesses | Available, but excluded by the portable Chariox contract | Available only with permissions that bypass Deno's sandbox | No v1 advantage for either runtime |
| Embedding into a Chariox target | Official C++ embedder API, unstable across major versions | Rust integration is natural, but a full Node-compatible Deno host is a larger contract than `deno_core` | Node embedding is feasible and preserves exact compatibility |

The sandbox is therefore not a reason to choose Deno. With OS containment held constant, Node gives Chariox the easier developer integration. Use Node's permission mode and module policy for an extra layer, not as the authority.

## TUI projection of the Environment browser

Yes, a TUI can open the user's default local browser as a viewer and control surface for the same Chromium instance managed by the Room Environment.

This behavior already exists for slice screens. `/slice screen` asks the kernel for a display endpoint, prints it, and uses `open`, `start`, or `xdg-open` to launch the OS browser ([slice command](../apps/cli/src/slice-command-handlers.ts); [external URL opener](../apps/cli/src/external-url.ts)). The browser tab is a viewer client. The controlled Chromium remains in the slice. Today, its graphical path is Chromium and Xvfb through x11vnc and websockify/noVNC ([slice display setup](../apps/kernel/slice-linux-docker/docker/slice-screen.sh)).

The Cloud View tab requests the same display endpoint and places it in a sandboxed iframe ([Cloud view coordinator](../../arroba-cloud/apps/web/src/terminal/view-route-coordinator.ts); [Cloud View mount](../../arroba-cloud/apps/web/src/react/terminal/ViewRouteMount.tsx)). The iframe is only the HTML container for the viewer. The current graphical transport is RFB over WebSocket through noVNC, not CDP or a copied browser DOM. The browser plan replaces noVNC with Selkies over the existing relay transport while CDP remains the structured agent-control path ([browser and computer plan, display technology](BROWSER_COMPUTER_USE_END_TO_END_PLAN.md#display-technology)).

`/app open` should follow the same model:

1. The TUI asks the kernel to open or focus the App URL in the Room's controlled Chromium.
2. The kernel returns the Environment and Tab identity plus an expiring viewer URL.
3. The TUI opens that viewer URL in the user's default browser.
4. The viewer renders the same Environment display stream used by the Web terminal iframe.
5. Pointer and keyboard input return through the kernel-owned Actor and Action path.

For a remote TUI, the default browser still runs beside the TUI, while its viewer connects through the authorized relay display tunnel to the Environment's worker. The relay carries the opaque display and input traffic. It does not create another browser or become runtime authority.

This is a projection of the controlled Chromium instance in the meaningful sense: the user sees and operates the same pixels, tabs, profile, cookies, and state that the agent operates. It is not an independent rendering of the App frontend.

The App plan was inconsistent here before this review. It said the Web terminal directly embedded the App UI from an installation origin, while TUI opened an Environment Tab. Those were separate frontend instances. The revised plan makes both Web and TUI render the Environment viewer. The Web terminal embeds the viewer in an iframe; TUI opens the viewer in the default browser.

The Room Environment contract is still a design contract rather than a finished daemon protocol ([protocol, section 4.1.2](PROTOCOL.md#412-shared-room-environment-protocol-direction)). The current slice viewer uses `slice_id`, and `/app open` is not implemented. This is implementation work, not an architectural obstacle.

## Required implementation validation

Chromium already proves that a broker and restricted target can contain untrusted code on macOS, Linux, and Windows. Chariox does not need a separate three-platform feasibility spike before implementation.

Chariox still has to test its own App worker. Chromium's evidence cannot prove that Chariox applies its policy before Node starts, passes only safe handles, confines installation paths, implements its HTTP broker correctly, or assigns resource limits correctly. These are release tests for the implementation rather than a research gate.

Phase 1 implements and validates macOS and Linux. Windows ports the same contract and hostile-package corpus in Phase 2. The Phase 1 implementation is complete only when these hold on signed macOS and production Linux builds:

- A pure npm App reads its package, writes its private data, restarts, and reads the same data through ordinary `node:fs`.
- Attempts to read Chariox state, provider credentials, the user's home directory, another App's data, process environment, or inherited host handles fail at the OS boundary.
- Attempts to open raw sockets, listen on a port, attach an inspector, load a native addon, use FFI, or spawn a subprocess fail.
- A standard Fetch-based service client reaches only a manifest-approved destination through the kernel broker. Redirect, DNS rebinding, private IP, and credential-revocation cases fail correctly.
- Infinite CPU, memory growth, IPC flood, crash loops, and ignored cancellation affect only that App.
- A signed and notarized macOS build still runs V8 JIT inside Seatbelt. The Linux namespace and seccomp policies pass the same hostile package corpus.
- `/app open` from local and remote TUI shows the same Environment App Tab seen in the Web terminal and controlled by the agent.

Phase 2 repeats every applicable test on the signed Windows build and adds LPAC or AppContainer identity, ACL, Job object, reparse-point, junction, and process-mitigation cases.

An implementation failure requires fixing the App worker or its platform policy. Switching to Deno would not fix an OS containment failure. If embedded Node cannot be maintained inside the required sandbox, the runtime contract must become narrower while the sandbox guarantee remains.
