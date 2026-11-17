# Apps: any program

Plan, 2026-11-17. Accepted by Miguel on 2026-11-17, with the answers in section 13.
Plan only: no product code comes with it.
Brief: Miguel, 2026-11-17 ("any program as a Chariox App"). This area starts
right after Apps distribution.

Spike evidence: `/root/.codex/evidence/anyprogram/` (outside the repository).
Facts about other projects are their status on 2026-11-17.

## 0. The plan in one page

**Goal.** Any program written in a language that compiles to WASM runs inside
Chariox as an App, mostly as is, and keeps its own UI. One runtime (WASM in the
kernel), no native-process Apps, no per-OS implementations. The UI reaches
every client and is drawn there. Files, network, secrets and clocks go through
Chariox interfaces. Agents use the program through its accessibility tree and
its actions, not through computer use.

**How.**

1. A new **program world** next to today's handler world: the program has a
   `main` that runs for as long as it likes.
2. Two ways to build a program, both on stock Wasmtime, sharing every host
   service:
   - a **component program** (WASI 0.3 + Chariox interfaces), for new code and
     for runtimes we ship, such as the Node runtime;
   - a **Linux-ABI program**: the program is compiled as if for a small Linux
     (musl), and Chariox implements the Linux calls itself inside the worker.
     This gives real threads, processes, signals, sockets, `mmap`, ptys, so
     existing programs and toolkits build without source changes.
3. One **display interface** with three kinds of window content: a **scene**
   (vector drawing the client rasterises itself), **GPU** commands, and
   **pixels** (the universal fallback, on today's display transport). Next to
   it, one **accessibility tree** per window in the AccessKit schema. Scene
   plus tree is to a toolkit program what the DOM mirror is to a web page.
4. **Compatibility endpoints** so toolkits run unchanged: a Wayland endpoint, a
   D-Bus session bus with the desktop portals, AT-SPI. A Chariox program sees
   what a Flatpak app sees. Per-toolkit scene backends come later as quality
   upgrades.
5. **Electron first.** The main process runs in WASM on a Node-compatible
   runtime, `electron` is an ordinary module supplied by Chariox, each
   `BrowserWindow` is a private tab of the kernel browser mirrored to every
   client, and menus and dialogs become data that clients and agents act on.

**What the spikes showed** (section 9): real parallel threads work on stock
Wasmtime 50 without a fork (3.6-4.0x on 4 threads). Sockets work and are
denied by default. Signals work by guest polling at no measurable cost in a
real loop; the engine route is closed. A vector scene is 13x smaller than
meshes and its delta is tens of bytes per frame when little changes. Three
unchanged official Electron samples run their main process in Wasmtime with
correct IPC, menu and dialog round trips; main is ready in 8-14 ms; JavaScript
compute is 14.5x slower than V8 with JIT.

**Order and size** (section 8; estimates, in lane-weeks):

| # | Milestone | Lane-weeks |
|---|---|---|
| M1 | Electron: program world, Node runtime, Electron APIs, windows | 30-40 (+8-10 optional) |
| M2 | Display interface v1, client renderer, agent tools; egui, Slint, iced, ImGui | 14-18 |
| M3 | Linux-ABI programs: threads, processes, signals, sockets; SDK; terminal programs | 20-28 |
| M4 | Wayland, D-Bus portals, AT-SPI: SDL, GTK, Qt, wxWidgets unchanged (pixels) | 18-26 |
| M5 | Scene backends for GTK 4 and Qt | 16-24 |
| M6 | GPU, audio, media | 20-30 |
| M7 | Flutter, .NET/Avalonia, Compose, Java, Go, Tk/X11 | 50-80 |

Total 170-245 lane-weeks; 100-135 through M5. M2 and M3 can run in parallel
after the first part of M1.

**What stays impossible, and why** (section 12): code that exists only as
native binaries; JIT compilers that emit machine code; a web engine itself;
more than 4 GiB per process until memory64; raw packet access to the host
network without a virtual network or host privilege.

**Wasmtime is not forked.** We carry a musl port, an SDK, a process layer in
the worker and, later, small toolkit patch sets (section 11).

**This plan replaces** the decision of 2026-10-22 against wrapped binaries
(programs that need threads, raw sockets or a native GUI). That section of
`spikes/wasm/APPS_V2_SHAPE.md` is deleted.

Miguel's answers to the ten questions are in section 13.

## 1. Principles used

Every decision below names the principle that drove it.

- **P1** Simplest for the person.
- **P2** Agents do it themselves, end to end.
- **P3** Familiar patterns: use the shape people and programs already know.

Fixed by the brief: only WASM runs program code; one implementation for every
kernel OS; IO only through Chariox interfaces; nothing is off limits to fork.

## 2. What exists and what changes

| Today (Apps v2) | Program world |
|---|---|
| World `chariox:app/app@1.0.0`: handlers only (`register`, `call-tool`, `deliver-event`, `lifecycle`, `deliver-wake`) | World `chariox:program/program`: exports `run`, which returns when the program exits. Handlers stay available to a program as optional exports |
| 30 s cap per call | No cap on `run`. Epoch ticks only yield for fairness and deliver stop requests |
| 64 MiB default, 128 MiB maximum | Declared per program, default 1 GiB, up to 4 GiB (wasm32); memory64 later |
| Admission allowlist: files, HTTP client, clocks, random | Adds sockets, processes, threads, environment, stdio, display, audio, GPU, each as a capability the manifest declares and the person grants at install |
| One worker process per installation generation, JSON calls | One worker process per running program. All of its WASM processes and threads live inside it. A crash ends that program only |
| UI: an HTML bundle in a sandboxed iframe on each client | UI: windows on the display interface, drawn by a Chariox renderer on each client. Electron windows are web pages (section 7) |
| Ahead-of-time compile cache per runtime identity | The same cache; a program's modules are compiled at install, so a 60 MB module costs nothing at start |

Handler Apps stay as they are. A program is an App: same package, signing,
Store, install consent and update path (P1: one thing to install; P3). The
manifest gains `kind = "program"` and its capabilities.

## 3. Target architecture

```
 client (web, TUI default browser, iOS)            kernel host
 +--------------------------------------+          +---------------------------------------------+
 | program view (shipped by Chariox)    |          | kernel                                      |
 |  scene renderer   pixel/video decode |  relay   |  windows, input owner, clipboard, dialogs,  |
 |  semantic DOM (accessibility)        |<========>|  accessibility trees, agent tools, policy   |
 |  hidden input (IME), clipboard, DnD  | encrypted|        ^                                    |
 | DOM mirror view (Electron windows)   |          |        | worker wire                         |
 +--------------------------------------+          |  +-----+---------------------------------+  |
   agents: chariox.program tools ----------------->|  | program worker (one OS process)       |  |
   (the kernel routes them to the worker)          |  |  Wasmtime: WASM processes and threads |  |
                                                   |  |  process layer, VFS, network policy,  |  |
                                                   |  |  display, Wayland, D-Bus endpoints    |  |
                                                   |  +---------------------------------------+  |
                                                   |  kernel browser (Electron windows only)     |
                                                   +---------------------------------------------+
```

The kernel stays the authority (runtime rule 1): windows, input ownership,
clipboard, dialogs and trees are kernel state, not client state. A program
runs and keeps its windows with no client attached.

### 3.1 The program world

Sketch (final names are set in M1):

```wit
package chariox:program@1.0.0;

world program {
  include wasi:cli/imports@0.3.0;        // args, env, stdio, clocks, random, files
  import wasi:sockets/...@0.3.0;         // TCP, UDP, DNS: allowed by kernel network policy
  import wasi:http/...@0.3.0;            // the existing broker
  import chariox:app/...;                // secrets, connections, file grants, events, store
  import chariox:process/...;            // spawn, wait, pipes, pty, signals
  import chariox:display/...;            // section 3.3
  import chariox:audio/...;  import chariox:gpu/...;
  export wasi:cli/run@0.3.0;             // main
}
```

- `run` is `main` (P3). Anything that builds as a WASI command starts here.
- **Lifetime.** A program starts when a person or an agent opens it, or at
  install when the manifest says `background`. It runs until it exits, the
  person closes its last window, or the kernel restarts. It keeps running with
  no client attached, so the same window is there on the next device (P1) and
  agents can work alone (P2). There is no snapshot of a running program:
  Wasmtime has none, and we do not build one. A kernel restart is a cold start;
  programs restore from their own files, as on a desktop (answer Q3).
- **Stopping.** Stop is a request first (`SIGTERM` or a close event), then an
  epoch interrupt after a grace period. The worker process is the hard limit.
- **Files** (P3: a small Linux layout):

  | Path | Content |
  |---|---|
  | `/app` | The package, read-only |
  | `/data` | Persistent data of this installation; `HOME=/data`, XDG directories below it |
  | `/tmp`, `/run` | Scratch, cleared on start; `/run` holds the Wayland and D-Bus sockets |
  | `/usr`, `/etc` | Runtime packs: fonts, certificates, time zones, locales, base tools |
  | `/mnt/<grant>` | Files and folders the person granted (`docs/APP_EXTERNAL_FILES.md`) |
  | `/dev`, `/proc/self` | `null`, `zero`, `urandom`, `tty`, `pts`; a minimal `/proc` (Linux-ABI programs) |

- **Environment.** From the manifest, plus `HOME`, `PATH`, `LANG`, `TZ`,
  `XDG_*`, `WAYLAND_DISPLAY`, `DBUS_SESSION_BUS_ADDRESS`. Locale and time zone
  follow the active client.
- **Fairness.** Each program has its own worker process and OS limits. Epoch
  ticks make busy threads yield; they do not end them.

### 3.2 Two program kinds, one set of host services

**D1. Component programs and Linux-ABI programs (P3).**

| | Component program | Linux-ABI program |
|---|---|---|
| Is | A WASM component: WASI 0.3 plus `chariox:*` interfaces | Core WASM modules built for `wasm32-linux-musl` whose Linux system calls are WASM imports |
| For | New code, small tools, runtimes Chariox ships (Node runtime), pure-Rust toolkits | Existing programs and toolkits: Qt, GTK, SDL, CPython, anything with threads or subprocesses |
| Threads | Cooperative only (upstream state) | Real parallel threads |
| Processes, signals, `mmap`, `epoll`, AF_UNIX, pty | Through `chariox:process`, limited | Yes, Linux shapes |
| Toolchain | Upstream as is (wasi-sdk, Rust `wasm32-wasip2`/`p3`) | Chariox SDK: clang + our musl port + libc++; Rust tier-3 target with prebuilt std |
| Upstream alignment | Full | When shared-everything threads reach components, we move over and delete our ABI |

Why two: the Component Model cannot run parallel threads in one component
today (shared-everything threads is Phase 1 and unimplemented; wasi-threads
was removed in Wasmtime 47). Core modules can (spike S1). And existing
programs expect Linux: `epoll`, `fork`/`exec`, signals, `/proc`. Telling every
build system "this is a small Linux" is what makes "mostly as is" true; a pure
WASI target needs a port of every program (no mainstream C, C++ or managed
toolkit has one).

Why not pass the calls to the host kernel, as WALI and lind-wasm do: that is a
per-OS implementation and a sandbox hole. The **process layer** implements the
calls inside the worker in Rust, on the same VFS, network policy and display
services the component programs use. It is written once and behaves the same
on a Linux, macOS or Windows kernel host.

Both kinds use one contract. `chariox:*` interfaces are defined in WIT.
Component programs import them directly. Linux-ABI programs reach them through
file descriptors (the display is a socket, like Wayland) and thin C bindings.

Details of the process layer are in section 4.

### 3.3 The display interface

**D2. Wayland-shaped windows; content is a scene, GPU commands or pixels; a
separate accessibility tree (P3).** Toolkits already have backends for exactly
this shape, so each port is small.

| Part | Shape |
|---|---|
| Surfaces | Toplevel windows, popups (menus, tooltips, combo lists) placed relative to a parent, subsurfaces. The kernel sends `configure` (size, scale, state, theme); the program commits frames; the kernel sends `frame-done` to pace drawing. With no client attached the pace drops to on-demand |
| Content: **scene** | A retained tree of layers. Each layer holds a display list: rectangles, rounded rectangles, paths, glyph runs, images, textured triangles, clips, transforms, opacity. Fonts and images are content-addressed and sent once. The program sends deltas. A subset of the Skia canvas model, so QPainter, Cairo, GSK, egui and Flutter display lists map onto it |
| Content: **GPU** | `chariox:gpu`: a WebGPU-shaped interface (the `wasi:webgpu` proposal's shape) and a GLES 3 shaped one. Section 4.5 |
| Content: **pixels** | Shared buffers plus damage rectangles. The kernel encodes them on today's display transport (lossless tiles for UI, video for motion). Works for every program |
| Input | Seat events: pointer, wheel, touch, keys with key codes and text, focus, modifiers. One input owner at a time (section 3.5). The client draws the cursor itself from the cursor shape |
| Text input (IME) | The program reports the focused text field, the caret rectangle and the surrounding text; the client focuses a hidden native input at the caret; composition arrives as pre-edit and commit events (the Wayland text-input shape). This also raises on-screen keyboards on phones |
| Clipboard | The program offers data by MIME type; the kernel holds one clipboard per person; each client bridges it to the device clipboard on a user gesture; agents read and write it by tool |
| Drag and drop | Inside a program: ordinary input. From the device: dropped files are uploaded as a file grant and arrive as paths under `/mnt`. To the device: the program's offer becomes a download or a native drag |
| Dialogs | Open file, save file, pick folder, native message box, print (to PDF), open link. Each is one kernel-owned request shown on every client, answered by a person or an agent (the rule-5 pattern). The program gets a granted path. Message boxes the toolkit draws itself are ordinary windows |
| Menus, tray, notifications | An application menu and tray menu as data; Chariox draws them in its own chrome on each client and offers them to agents as actions. Notifications go to Chariox notifications |
| Accessibility | One tree per window, AccessKit schema, sent as updates, with actions back (click, focus, set value, scroll into view, expand). Section 6 |
| Output | Size, scale factor, colour scheme, locale of the active client |

Why a scene and not a common widget tree: programs keep their own UI. A widget
tree would redraw every toolkit in Chariox's look and break custom widgets.
The scene carries how it looks; the accessibility tree carries what it means.
Together they are what the DOM mirror gives for web pages: structure, not
pixels, drawn natively at the client.

Why three content kinds: the scene is small and sharp at every client scale,
but only toolkits with a display-list path can produce it. GPU covers 3D,
games and GPU-only toolkits. Pixels cover everything from day one, at a
bandwidth cost. A window may mix them (a video subsurface inside a scene
window).

Measured (spikes S3, S4): a vector scene for a busy egui window is 17 kB per
full frame and 12-77 B per frame as a delta when little changes; the same
frame as meshes is 228 kB. ImGui draw lists are 157 kB per frame raw and
23-1,038 B as a delta. So the wire carries shapes and deltas, never meshes,
and scrolling must be a layer transform, not re-sent content.

### 3.4 How clients render

**D3. One program view, shipped by Chariox, in every client (P1, P3).**

- **Web, local TUI and remote TUI** (the TUI opens the default browser, as for
  App views): a Chariox web bundle. The scene is rasterised with CanvasKit
  (Skia as WASM) at the client's own scale factor. Glyph runs need drawing by
  glyph id, which Canvas 2D cannot do. Pixels and video decode with the
  existing display-transport client. Beside the canvas, a **semantic DOM**
  built from the accessibility tree makes the window work with the client's
  screen reader, and a hidden input carries IME. This is the architecture of
  Flutter web (canvas + semantics tree + hidden input) (P3).
- **iOS:** the same bundle in a WKWebView first. A native renderer (Skia or
  CoreGraphics drawing glyph runs, UIAccessibility from the tree) comes later;
  the wire format does not change.
- **Electron windows** use the existing DOM mirror view (section 7).
- The client downloads the renderer once and caches it. It never downloads the
  program: a 60 MB Qt program costs the client only its scene.
- The program view is Chariox code. It needs no per-app sandbox: it runs no
  program code, only draws data.

### 3.5 Multi-window and multi-client

**D4. One program instance, shared by every client of the person (P1).**

- A window exists once, in the kernel. Every client shows the same window. This
  is the kernel-browser model, not the "each client has its own view instance"
  model of App views: an unchanged program has one UI state.
- **Size.** The window is sized for the client that owns input. When another
  client takes over, the kernel reconfigures the window to that client's size
  and scale; programs handle resize natively. Passive clients see the window
  scaled to fit (answer Q2).
- **Input.** One input owner at a time, with explicit takeover and a ledger,
  as in the kernel browser. An agent acting through the tree does not take the
  pointer.
- **Windows on a client.** Chariox is the compositor. A toplevel fills the
  program view; several toplevels are tabs; transient dialogs are centred
  overlays; popups are placed relative to their parent inside the view. This
  is how phone Wayland compositors treat desktop programs (P3).
- **No client attached.** The program draws to a virtual output of 1280x800,
  the size the kernel browser uses. The kernel keeps the last scene and tree.

### 3.6 Latency and bandwidth budget

The rule of 2026-10-14 applies unchanged: **every echo gate is P95 ≤ measured
wire RTT + 60 ms, and the measured RTT is recorded next to every P95.**

| Gate | Target |
|---|---|
| Local (client on the kernel host): input → paint | P95 ≤ 50 ms; animation ≥ 55 fps in the scene tier |
| Remote: key or click → visible change | P95 ≤ RTT + 60 ms |
| Window open, renderer cached | P95 ≤ RTT + 250 ms to first full frame |
| Device switch: takeover → resized frame | P95 ≤ RTT + 250 ms |
| Reconnect, kernel reachable | Last frame stays visible; live again P95 ≤ RTT + 500 ms |
| Agent action → tree update visible to the agent | P95 ≤ 100 ms on the kernel host |

Where the 60 ms goes, scene tier:

| Step | Budget |
|---|---|
| Client: capture and send input | 2 ms |
| Kernel: route to the worker | 3 ms |
| Program: handle the event and commit a frame (measured under 1 ms per frame for egui and ImGui in Wasmtime; the program draws on input, not on the next tick) | ≤ 16 ms |
| Worker and kernel: diff, encode, send | 4 ms |
| Client: decode, rasterise, present (one display frame at worst) | ≤ 20 ms |
| Slack | 15 ms |

Pixels add encode and decode (5-10 ms each); UI must use lossless tiles to
stay inside the gate. Kernel-side GPU adds one video frame; a cell that misses
the gate is reported as a miss, not waived.

Bandwidth targets: scene tier ≤ 10 kbit/s idle and ≤ 2 Mbit/s P95 while
interacting, after compression; pixels tier 0.5-8 Mbit/s, the display
transport's range; tree updates coalesced to at most 10 per second per window.
Typing while scrolling a 200-line editor measured 11 kB per frame raw before
layer scrolling and compression, which is the case the layer model removes.

## 4. Runtime gaps and how each is closed

Summary. "Upstream" is Wasmtime, WASI and the WASM proposals.

| Gap | Upstream today | Upstream later | What we do | Fork? |
|---|---|---|---|---|
| Long-running `main` | `wasi:cli/run`, async since WASI 0.3 (default since Wasmtime 46) | Stable | Use it; lift the call cap | No |
| Parallel threads | Core `threads` on; `SharedMemory` off by default, tier 2. wasi-threads removed in 47. Components: cooperative threads only | Shared-everything threads: Phase 1, no date. Cooperative threads default in wasi-sdk planned end of 2026 | Own thread spawn in the process layer: one instance per thread over one shared memory (4.1) | No |
| Processes | Nothing, and no proposal | Nothing planned | Processes are instances in the same worker; `posix_spawn` and `vfork`+`exec` unchanged, every `fork` as a build flavour (4.2) | No |
| Signals | Nothing | Nothing planned | Guest polling of a pending word; host handles `KILL`/`STOP` (4.3) | No |
| Sockets | `wasi:sockets` TCP, UDP, DNS; denied by default since 48. No raw, no AF_UNIX | `wasi-tls` Phase 1 | Kernel network policy in front of host sockets; loopback and AF_UNIX virtual; virtual network stack for raw sockets (4.4) | No |
| GPU | `wasi:webgpu` Phase 2; out-of-tree `wasi-gfx-runtime` 0.5.2 for Wasmtime 46-50 | Unclear | `chariox:gpu` in the same shape, on wgpu; GLES shape beside it (4.5) | No (a dependency we may have to vendor) |
| Audio, clipboard, windows | Nothing | Nothing planned | `chariox:audio`, `chariox:display` | No |
| `mmap`, shared mappings | Linear memory only | Multi-memory exists; no file mapping | Emulated in linear memory (4.6) | No |
| Dynamic loading | Closed world; `dylink.0` convention without stable ABI; no embedder `dlopen` | Unclear | Static linking first; `dlopen` of side modules in the process layer later (4.7) | No |
| More than 4 GiB | memory64 implemented, off by default | Expected on | Enable per program when a validation program needs it | No |
| `setjmp`, C++ exceptions | WASM exceptions, default since 47 | Stable | Use | No |
| Snapshot of a running program | None | None | Not built; programs restore from their files | No |
| JIT inside the guest | Impossible by design | - | Interpreters and ahead-of-time builds; "JIT emits WASM" through 4.7 later | No |

Wasmtime patches we considered and reject for now: re-entering the guest from
the epoch callback for signals (spike S6: the callback gets no mutable store);
parallel threads inside a component; live fork of a store. Each would be a
permanent fork of a monthly-released engine. The routes above need none.

### 4.1 Threads

**D5. Instance per thread over one shared memory, in the process layer (P3:
the wasi-threads and Emscripten model).** `pthread_create` asks the host to
instantiate the module again on a new OS thread with the same `SharedMemory`
and a fresh stack and TLS block. Spike S1: stock Wasmtime 50, a 67-line host,
4 threads give 3.69x, 4.02x and 3.63x speed-up (native 3.96x).

Costs and limits:

- Wasmtime calls shared memory "known to not be sound" for host code that
  reads guest memory as ordinary slices. The process layer therefore never
  borrows guest memory: every access is an atomic-safe copy. This is a rule
  with a check, and spike S15 audits it.
- Tier 2 in Wasmtime and no pooling allocator for shared memories: each
  program reserves its maximum memory up front. Fine with one worker per
  program.
- Function tables are per instance. Static programs are unaffected; `dlopen`
  must replay table growth in every thread (4.7).
- Futexes, thread exit, robust mutexes, thread-directed signals are ours to
  implement and test (pthread conformance from the Linux Test Project).

Component programs keep upstream cooperative threads. A component that needs
cores is built as a Linux-ABI program.

### 4.2 Processes

**D6. A process is a set of instances in the same worker with its own memory,
file table and pid (P3).**

- `posix_spawn` and `vfork`+`exec` are supported without instrumentation. A
  `vfork` child runs on the parent's stack and memory with the parent
  suspended until the child calls `exec` or exits, so no stack copy is needed.
  `exec` loads another WASM module from `/app` or a runtime pack. `vfork` is
  built on the WASM exception-based `setjmp` in our musl. This mechanism is not
  proven yet: spike S13 builds it and validation cell AP3-10 proves it. Its
  kill criterion: if `setjmp` cannot return a second time into the parent's
  frames without a stack copy, `vfork` joins the fork flavour and only
  `posix_spawn` stays uninstrumented.
- **Every `fork`, with or without `exec`,** needs the fork flavour. The child
  continues from the call with its own copy of the stack, and it runs code
  before any `exec` (a shell's `dup2`, `chdir`, `setsid`; Python
  `multiprocessing` with the fork start method; pre-forking servers). That is
  a build flavour: the SDK post-link tool instruments the binary (Binaryen
  Asyncify, about 50% larger and usually up to 20% slower, or a lighter
  fork-only instrumentation as Kandelo did). The memory copy is real, not
  copy-on-write. A binary built without the flavour that calls `fork` fails
  with `ENOSYS` and a message that names the flavour.
- `exec` of a native binary is refused, always. A program that shells out to
  `git`, `sh`, `python` or `ffmpeg` needs those tools as WASM. Chariox ships
  them as **runtime packs** a manifest depends on (P3: Flatpak runtimes).
  `xdg-open` and friends go to the portal and open on the client.
- Pipes, `socketpair`, AF_UNIX with fd passing, `epoll`, `eventfd`, `timerfd`,
  `inotify`, ptys, sessions and process groups, `waitpid`, exit codes: all
  inside the process layer.

### 4.3 Signals

**D7. Signals are delivered by the guest polling a pending word (P3: WALI's
safepoints).** The SDK post-link tool inserts a load-and-branch at loop
headers and function entries; musl checks the word at every system call
return. Spike S6: the handler ran 0.2-1.2 ms after `raise` in a loop that
never calls the host; the poll cost on a real loop is inside measurement noise
(-2.1% to +1.7%); the worst case is a poll written in the source of a loop
the compiler would otherwise fold away (8x), which is why the tool inserts
polls after optimisation, on the linked binary.
`SIGKILL` and `SIGSTOP` are done by the host with an epoch interrupt. Blocking
calls return `EINTR`. `SIGSEGV` from a trap is reported to the process as a
fatal signal; handlers that resume after a fault are a named limit.

### 4.4 Network

**D8. Plain sockets, allowed by the kernel's network policy (P3).**

- TCP, UDP and DNS: implemented on host sockets behind a policy check per
  connect, bind and send, in the worker. The manifest declares what the
  program needs (hosts, ports, "any host", inbound listen); the person grants
  it at install (answer Q4). Spike S2: stock Wasmtime refuses sockets
  without a grant and passes TCP listen/accept/echo, name lookup, an outbound
  HTTP request and UDP with one.
- **Loopback and AF_UNIX are virtual and private per program.** Two programs
  can both listen on `127.0.0.1:3000`, and nothing on the host can reach them.
  A person opens a program's local web port through the kernel browser, by an
  explicit action.
- TLS is the program's own (OpenSSL compiled to WASM) on a TCP socket. The
  HTTP broker with Vault-held credentials stays available for programs that
  use it.
- **Raw and packet sockets:** phase 1 gives unprivileged ICMP echo. Phase 2
  gives each program a virtual network interface on a user-space TCP/IP stack
  in the worker (the gVisor and slirp pattern): raw sockets, packet capture of
  its own traffic and odd protocols work inside the virtual network, and only
  policy-checked TCP, UDP and ICMP leave it. Raw frames on the host's real
  network need a host privilege and are a named limit.

### 4.5 GPU

**D9. `chariox:gpu` in the shape of WebGPU, plus a GLES 3 shape; run in the
kernel first, replayed on the client later (P3).**

- WebGPU shape: follows the `wasi:webgpu` proposal so we can converge; host
  side on wgpu, starting from `wasi-gfx-runtime`.
- GLES 3 shape (WebGL 2 semantics): what Qt, GTK, SDL and most engines can
  emit today, and what a browser client can replay directly.
- Vulkan-only programs: a translation layer in the guest, later, not promised
  in M6.
- **Phase 1, kernel side:** commands run on the kernel host's GPU, or in
  software where there is none, and the result goes out as pixels or video.
  Works for every client and for agents with no client.
- **Phase 2, client side:** the command stream is replayed on the active
  client's GPU (the Dawn Wire pattern), with a shadow of resources in the
  kernel so another device can take over. Readbacks cost a round trip. This is
  the only way to get interactive 3D from a kernel host without a GPU
  (answer Q8).

### 4.6 Memory, files, clocks

- `mmap` of anonymous memory and private file mappings is emulated inside
  linear memory. A shared file mapping is coherent inside one process; between
  processes it is coherent at `msync` and unmap. Programs that need live
  shared memory between processes (few) are a named limit until multi-memory
  sharing is designed.
- The VFS is the worker's: package, data, grants, runtime packs, `/proc`,
  `/dev`. File watching (`inotify`) is served by the VFS.
- Clocks and timers are host clocks. Random is the host's CSPRNG.
- Secrets: the keyring interfaces (the Secret portal, `libsecret`, Electron
  `safeStorage`, `keytar`) store into a per-installation area sealed by the
  kernel Vault. A program reads back what it stored. Vault credentials of the
  person reach a program only by Vault fill or an explicit grant, and never
  reach the model (section 6.4).

### 4.7 Dynamic loading and plugins

Static linking first: one closed-world module per executable. Then `dlopen`
of position-independent side modules, linked by the process layer (the
`dylink.0` convention, as lind-wasm and Wasmer do), replayed into every thread
instance (spike S14). This is what Python extension modules, Qt plugins and
Node native addons need. The same facility later lets a guest JIT emit WASM
instead of machine code.

### 4.8 Audio

**D10. `chariox:audio`: PCM streams out and in, mixed in the kernel, Opus to
clients (P3).** Playback latency gate: ≤ RTT + 60 ms plus a 40 ms jitter
buffer, in sync with video by timestamp. Microphone and camera need the
client's permission and a Chariox permission prompt. Because everything is
recompiled, compatibility is a library in the SDK sysroot, not a daemon: our
`libpulse`, `libasound` (PCM subset) and SDL audio driver speak
`chariox:audio`.

## 5. Toolkits and languages

### 5.1 Strategy

**D11. Unchanged first, native scene second (P1, P3).** No mainstream C, C++
or managed toolkit has a WASI port, and each has a different backend API.
Porting them one by one before anything runs would take years. So:

1. **Run the toolkit's existing Linux backend unchanged** against the Wayland,
   D-Bus and AT-SPI endpoints (M4). Content is pixels. Every GTK, Qt, SDL and
   wxWidgets program works at this point, with an accessibility tree.
2. **Then add a scene backend per toolkit** where its drawing model allows
   (M5): same program, sharper and far less bandwidth.

Pure-Rust toolkits skip step 1: they have no Wayland dependency worth keeping
and their integration layers are small, so they go straight to the display
interface as component programs (M2).

Kandelo (Automattic) already runs libwayland, GTK 3, qtbase and SDL 2/3
compiled to WASM with musl on its own kernel in a browser (23 of 24 cases).
That is proof the toolkits compile; its code is GPL and is not used.

### 5.2 Toolkits, in order

Sizes are lines of code of existing backends of the same kind, as a measure of
the work.

| Order | Toolkit | First route | Scene route | Accessibility tree from | Size precedent | Fork? |
|---|---|---|---|---|---|---|
| M1 | Electron (web UI) | Kernel browser tab + DOM mirror | The DOM mirror is its scene | The DOM | Section 7 | No |
| M2 | egui | Integration crate: shapes → scene | Same | AccessKit, built in | eframe-style crate, about 1.5k | No |
| M2 | Slint | Its `Platform` API with the software renderer → pixels with damage | Renderer emitting scene items | AccessKit (from its winit backend) | Platform API is public | No |
| M2 | iced, and winit programs in general | A winit backend for Chariox; tiny-skia → pixels | Primitive list → scene | AccessKit where the toolkit has it | iced_winit 4,420 | winit patch set until backends can live out of tree |
| M2 | Dear ImGui | Platform + renderer backend pair: draw lists as scene deltas | Same | None (actions; optional test-engine hooks) | null 102, sdl3 890, opengl3 1,200 | No (backends are out of tree by design) |
| M4 | SDL 3 (SDL 2 through sdl2-compat) | Its Wayland driver, pixels; GLES in M6 | GPU tier | None | emscripten driver 3,030; offscreen 897 | No |
| M4 | GTK 4 | Its Wayland backend with the Cairo renderer | M5: a GSK renderer that sends render nodes (Broadway did this: GDK 12,901 + GSK 1,104) | AccessKit backend, in GTK since 4.18 (2,840) | - | M5 only: in-tree renderer, a patch set |
| M4 | GTK 3 | Wayland + Cairo, pixels | None planned | ATK → AT-SPI | - | No |
| M4 | Qt 6 Widgets and Quick | Its `wayland` platform plugin, pixels; Quick in software | M5: a `chariox` platform plugin with a recording paint engine; Quick through the software adaptation, then RHI on `chariox:gpu` | AT-SPI first; QAccessible → AccessKit in the plugin | minimal 359, offscreen 1,639, vnc 2,224, wasm 10,580 (accessibility 1,619) | No (plugins are out of tree; private API, pinned to Qt LTS) |
| M4 | wxWidgets | wxGTK: it is a GTK 3 program | - | ATK → AT-SPI | - | No |
| M3 | Terminal UIs (ncurses, ratatui, ...) | pty → Chariox terminal surface | - | Terminal text | - | No |
| M7 | Flutter | dart2wasm + the engine's web layer with a Chariox renderer backend → scene; text layout by Skia Paragraph as WASM | Same | Flutter semantics tree → AccessKit | embedder GLFW 4,092; flutter-pi 39,387 | Yes: engine web layer |
| M7 | Avalonia (.NET) | Own windowing platform + Skia as WASM → pixels | Drawing-context recorder → scene | Automation peers → AccessKit | Headless 2,522; LinuxFramebuffer 3,346; Browser 8,168 | No (backends are packages) |
| M7 | Compose Multiplatform | Kotlin/Wasm + Skiko with Chariox bindings | Same as Flutter | Compose semantics → AccessKit | - | Skiko patch set |
| M7 | Swing, JavaFX | OpenJDK Zero interpreter; AWT Wayland toolkit; JavaFX Glass headless/Monocle → pixels | None planned | Java accessibility → AT-SPI | Glass headless 1,725; Monocle 17,045 | OpenJDK patch set |
| M7 | Tk, Xlib programs | Xwayland compiled to WASM as a sibling process, pixels | None | None | Tk Xlib emulation 11,285 | No |
| after M3 | Tauri | Same mapping as Electron: Rust backend as a program, webview = kernel browser tab | - | The DOM | Small API | No |

Miguel's list is covered: Qt (M4, M5), GTK (M4, M5), Flutter (M7), egui (M2),
Slint (M2), SDL (M4, M6), wxWidgets (M4), ImGui (M2), Avalonia (M7).

### 5.3 Languages

| Language | Route | Status today | Gap we close | When |
|---|---|---|---|---|
| JavaScript, TypeScript, Node | Chariox Node runtime, a component (QuickJS-ng through wasm-rquickjs 0.5.3) | 73% of the Node test subset upstream; no JIT | Electron module, speed (section 7.6) | M1 |
| Rust | Component: `wasm32-wasip2`, tier 2, in use in this repo. Linux ABI: `wasm32-wali-linux-musl`, tier 3 | Works; tier-3 target has no prebuilt std | SDK ships a prebuilt std for the pinned compiler | M2, M3 |
| C, C++ | Component: wasi-sdk 34. Linux ABI: Chariox SDK (clang, our musl, libc++ with exceptions and threads) | LLVM has the `wasm32-linux-muslwali` triple since 22 | The musl port and sysroot | M2, M3 |
| Python | CPython, Linux-ABI build | WASI is tier 2 upstream, single thread, static | Threads, `subprocess`, sockets, `ctypes`, extension modules (4.7) | M3 |
| Go | wasip1 only upstream: one thread, no outbound sockets in std. TinyGo: wasip2 | Small programs work as components | A Go port to the Linux ABI (runtime, `syscall`, netpoll): a fork of the Go toolchain | M7 |
| .NET (C#, F#) | NativeAOT-LLVM (componentize-dotnet), or the Mono interpreter on the Linux ABI | Experimental, no threads | Threads, the Avalonia backend | M7 |
| Java, Kotlin/JVM, Scala | OpenJDK Zero (C++ interpreter) on the Linux ABI | No JVM targets WASI today | The port; interpreter speed only | M7 |
| Kotlin/Wasm | WASM GC + exceptions (Wasmtime default since 47) | Beta, browser-first | Non-browser bindings | M7 |
| Swift | Swift SDK for WASM (wasip1, wasip1-threads) | Works without UI toolkits | Runs on the thread host as is | M3 |
| Dart | `dart compile wasm`, WASM GC | Runs on Wasmtime standalone | Flutter (5.2) | M7 |
| Zig | WASI targets | Works as a component | Linux ABI through clang for C dependencies | M2, M3 |
| Ruby, PHP, Lua, Perl, Tcl | C interpreters | wasip1 builds exist for Ruby, PHP, Lua | Built with the SDK, shipped as runtime packs | M3 |
| Haskell, OCaml | GHC WASM backend (wasip1); OCaml needs WASM GC | Works for command-line programs | None planned | - |
| Shell | dash and bash in the base runtime pack | bash needs the fork flavour | 4.2 | M3 |

A wasip1 command module is wrapped as a component at pack time with the
upstream adapter. A wasip1-threads module (Rust tier 2, Swift) runs on the
thread host directly: spike S1 used exactly that target.

Speed to expect: compiled code about 1-2.5x native CPU (S3: 2.1-2.3x; S4:
0.9-1.7x; WALI's paper: about 2x). Interpreters lose their JIT: QuickJS was
14.5x slower than V8 on a compute micro benchmark (S5b) and about 6x on mixed
work (the repository's npm-compat spike).

## 6. Agents

**D12. One tree format, one tool set, shaped like the kernel browser tools
(P2, P3).**

### 6.1 The accessibility tree

AccessKit's schema is the single format: roles, names, values, states, bounds,
relations, actions. The kernel holds the current tree of every window, with or
without clients.

| Source | How it becomes the tree |
|---|---|
| egui, Slint, GTK ≥ 4.18 | They already emit AccessKit updates; an AccessKit adapter for Chariox forwards them |
| Qt | AT-SPI over our D-Bus first; then QAccessible → AccessKit inside the `chariox` plugin (the Qt WASM bridge is 1,619 lines) |
| GTK 3, wxWidgets, Java | AT-SPI over our D-Bus, converted in the endpoint (spike S18) |
| Flutter, Compose, Avalonia | Their semantics trees, converted in the port |
| Electron | The DOM, through the existing kernel browser snapshot |
| SDL, ImGui, games | No tree. Actions and section 6.5 |

Spike S4: a busy egui window is 332 nodes; updates are 0-1.2 kB except while
scrolling, where every node's absolute bounds change (217 kB as JSON). So the
wire format is binary, bounds are relative to the scroll container, and
updates are coalesced.

### 6.2 Tools

`chariox.program`, with the request and result shapes of
`chariox.kernel_browser` so agents learn nothing new: list windows; snapshot
(refs, capped at 5,000 nodes); find; act on a ref (click, focus, set value,
select, expand, scroll into view, press keys, type text); wait for a
condition; screenshot (rasterised in the kernel from the scene, no client
needed); list and call actions; answer a dialog; read and write the clipboard;
start and stop the program; read its output and exit code. Electron windows
use the kernel browser tools themselves, scoped to that program's windows.

Every action carries an actor receipt (agent or person), as App calls do
today. Tree actions do not take the input ownership of a person who is using
the window.

### 6.3 Actions

An action is a named operation with a typed input. Three sources:

1. **Declared.** A program may also export today's handler tools from the same
   component, or declare actions in its manifest. This is the SDK path for
   authors who want a first-class agent API.
2. **Derived, no work by the author.** Application menus and tray menus
   (Electron `Menu` templates, Qt platform menus, GTK `org.gtk.Menus` and
   `org.gtk.Actions` on D-Bus), other D-Bus interfaces the program exports
   (MPRIS and the like), keyboard shortcuts, and the program's own command
   line: an agent can run `program --help` and any subcommand as a process.
   Spike S5b showed the menu route: an unchanged Electron sample's menu arrived
   as data and a click on it by path ran the app's handler.
3. **Wrapped.** A packager adds actions as small scripts over the tree. Later.

### 6.4 Secrets

The existing rule holds: the product never hands a Vault value to the model.
An agent fills a field by Vault handle; the kernel types the value into the
program. From then on the kernel masks that node's value in snapshots and
blurs its rectangle in screenshots given to the model. Toolkits already hide
password fields in their trees. Nothing else is guarded: what a program shows
on its own screen is its business.

### 6.5 Programs without a tree

Games, ImGui tools and some SDL programs have no tree. An agent then has
actions (6.3) and, as a fallback, the screenshot plus pointer and keys inside
that program's window. That is computer use scoped to one program view; it is
allowed, marked in the receipt, and never the default when a tree exists
(answer Q9).

## 7. Electron: the first milestone

### 7.1 The mapping

An Electron app is three things. Each gets a home.

| Part | Runs today in | Runs in Chariox |
|---|---|---|
| Main process: the app's Node code | Node inside Electron | A WASM program on the **Chariox Node runtime** |
| Electron APIs (`app`, `BrowserWindow`, `ipcMain`, `Menu`, `dialog`, ...) | Electron's native code | The `electron` module, supplied by Chariox, written against Chariox interfaces |
| Windows: the web UI, the preload script | Chromium renderer processes | Web pages in a web engine. Recommended: private tabs of the **kernel browser**, mirrored to every client (7.3) |
| Child processes, `utilityProcess`, worker threads | OS processes and threads | More WASM instances of the Node runtime in the same worker |
| Native addons (`.node`) | Machine code | Replaced or recompiled (7.5) |

Chromium does not run in WASM and this plan does not try. The web UI is
JavaScript and HTML, and a web engine runs it inside that engine's own
sandbox, as for any web page. Everything that was native code moves to WASM.
The app is not rebuilt: a pack tool turns its directory or `asar` archive into
a Chariox package.

### 7.2 Main process

**D13. Adopt wasm-rquickjs as the base of the Chariox Node runtime (P3).**
Apache-2.0, active (0.5.3 on 2026-11-13), a WASI 0.3 component on the
Wasmtime 50 line, QuickJS-ng with Node's module surface implemented in Rust:
73.2% of its Node test subset (stream 713/753, fs 339/482, crypto 204/239,
net 150/223, http 240/898, child_process 40/208, worker_threads 25/189, tls
4/207, `node:sqlite` 36/39). The weak areas are where Chariox supplies the
service anyway (processes, threads, sockets, TLS).

Spike S5b: the runtime built as a 7.0 MB component in 54 s. With a 214-line
`electron` module resolved from `node_modules`, with no change to the runtime,
the three official IPC samples of Electron 45 ran unchanged: renderer → main,
main → renderer from a menu click, and `dialog.showOpenDialog` answered by a
client. Main was ready in 8-14 ms. Peak memory of the whole process was
45-59 MB.

`process.platform` reports `linux` (P3). Apps have Linux code paths, and the
census shows them calling `xdg-open`, `xdg-mime` and `gsettings`, which the
portals answer.

### 7.3 Windows

**D14. A `BrowserWindow` is a tab of the kernel browser in a browser context
private to that installation; clients see it through the DOM mirror (P1, P2).
Answer Q1.**

Why the kernel and not each client's own web engine:

1. **Unchanged renderers need Chromium.** The census of 12 real apps (S5a): 11
   use `setWindowOpenHandler` and load non-literal URLs, 7 `executeJavaScript`,
   6 `protocol.handle` and 6 session cookies or storage, 5 `webRequest`, 4
   `printToPDF`, 4 `sendSync`, 4 `@electron/remote`, 4 have windows with Node
   integration. All of these are a few DevTools-protocol calls on a Chromium
   we control, and hard or impossible in a sandboxed frame on a client,
   least of all in WebKit on iOS.
2. **Agents need a renderer with no client attached.** The tree of an Electron
   app is its DOM. A kernel-side page is there when no device is open, and the
   existing `chariox.kernel_browser` tools already read and drive it (P2).
3. **One window state for the person.** Most apps keep state in the renderer.
   A kernel-side page is the same document on the laptop and the phone. A
   page per client would be two unrelated instances, like two browser tabs
   (P1).
4. **It is the same model as every other program in this plan:** the program
   runs in the kernel and its UI is sent as structure. The DOM mirror is the
   precedent the brief names.

How each piece maps:

| Electron | On the kernel browser |
|---|---|
| `loadFile`, `file://`, custom schemes, `protocol.handle` | Request interception answers from `/app` or calls the app's handler in main |
| Preload, `contextBridge` | Injected before page scripts, with the app's preload source unchanged inside a wrapper that supplies `require('electron')` |
| `ipcRenderer.send`/`invoke`, `webContents.send`, `MessagePort`s | Protocol bindings between the page and the worker; no TCP |
| `ipcRenderer.sendSync`, `@electron/remote` | A blocking request from the page, answered by main (main is another process, so the page may wait) |
| `nodeIntegration`, unsandboxed preloads | A `require` in the page whose Node modules are proxies to a companion Node instance in the worker, over the same blocking bridge. Slower per call; correctness first |
| `webRequest`, `session.cookies`, cache, permissions, certificate handlers | The matching protocol domains on the program's own browser context |
| `setWindowOpenHandler`, child windows, `WebContentsView`, `<webview>` | More tabs or frames of the same context, composed by the mirror view |
| `printToPDF`, `capturePage`, zoom, find, devtools | Direct protocol calls |
| Remote sites the app loads (Mattermost, Element) | Loaded by the kernel browser under the program's network grant |

Isolation: a program's browser context shares nothing with the person's own
browsing: no cookies, no storage, no logins. Its storage lives in the
program's `/data`. Its network goes through the program's network grant.

Cost: one Chromium tab per window on the kernel host (roughly 150-400 MB), and
the mirror's latency. The gate is the same rule, P95 ≤ RTT + 60 ms; today's
mirror targets are p50 ≤ 80 ms and p95 ≤ 120 ms for input, so M1 depends on
`docs/BROWSER_LATENCY_GAP_PLAN.md` for remote typing. Canvas-heavy pages fall
back to tiles or video, as in the browser today.

**Client-rendered fast path (optional, M1d).** For apps that qualify (context
isolation on, sandbox on, no sync IPC, no Chromium-control features), the
window's web bundle can instead run in each client's own web engine, like an
App view, with IPC over the kernel connection. It is faster to the eye and
uses no kernel memory per window, but: renderer state is per client, WebKit
clients run code written for Chromium, the page needs a real origin with a
service worker (an opaque-origin frame cannot have one), and agents still need
a kernel-side page. In the census 3 of 12 apps pass these conditions on their
static flags (minimal-repro, drawio-desktop, Simplenote). It is a later
optimisation, not the base.

### 7.4 Electron and Node APIs

| API | Becomes |
|---|---|
| `app` lifecycle, paths, version, single-instance lock, deep links, login items | Program lifetime; paths under `/data`; Chariox links; one instance per installation by construction |
| `BrowserWindow`, `webContents`, `WebContentsView` | 7.3. Bounds, show, hide, full screen, title map to Chariox window state |
| `Menu`, `MenuItem`, accelerators, `Tray`, context menus | Menus as data: drawn by Chariox chrome on each client, offered to agents as actions. Accelerators fire while the window has focus. `globalShortcut` is program-wide only |
| `dialog` (open, save, message box, error box) | Kernel-owned requests any client or agent answers |
| `shell.openExternal`, `openPath`, `showItemInFolder`, `trashItem` | Link opens on the client; files are in the VFS |
| `clipboard`, `nativeTheme`, `screen`, `powerMonitor`, `systemPreferences` | Display interface: clipboard, theme, output; honest constants elsewhere |
| `Notification`, badge | Chariox notifications |
| `session`, `net`, `protocol`, downloads | 7.3; `net` is the runtime's fetch under the network grant |
| `safeStorage`, `keytar` | A per-installation store sealed by the Vault |
| `utilityProcess`, `child_process.fork`, `worker_threads` | WASM instances of the runtime |
| `child_process.spawn` of other programs | WASM tools from runtime packs (`git`, `rg`, `pandoc`); never native binaries |
| `autoUpdater`, `crashReporter` | Off: Chariox updates the package; crashes go to the program log |
| `desktopCapturer`, vibrancy, Touch Bar, dock, taskbar, `serialport` | Not available; calls fail the way they do on a Linux desktop without the feature |

Census (S5a): 494 distinct Electron members across 12 apps; 219 are used by
one app only. Implementing the 197 members used by the four simplest apps
already covers 61-71% of what each later app uses (Mattermost: 39%, it uses
284). So the work is ordered by app, not by module.

### 7.5 Native addons and spawned programs

9 of 12 census apps load native addons and 9 of 12 spawn other programs.

| Tier | Route | Examples from the census |
|---|---|---|
| 1 | A maintained **addon map**: the pack tool swaps the addon for a module with the same API on Chariox services or an existing WASM build | `keytar` → Vault store; `sqlite3`, `better-sqlite3` → `node:sqlite`; `node-pty` → Chariox pty; `nodehun` → Hunspell as WASM; `onnxruntime` → its WASM build; `native-keymap` → the client's keyboard layout |
| 2 | Compile the addon's C or C++ source with the SDK against the N-API headers and load it as a side module (needs 4.7, so after M3) | `node-libcurl`, database drivers, `sqlite-vec`, `russh` |
| 3 | Not possible: hardware or OS-specific | `serialport`, `registry-js`, `cf-prefs` (the last two are not loaded on Linux anyway) |

### 7.6 The JavaScript speed risk

QuickJS has no JIT. S5b: `fib(27)` in main took 29 ms against 2 ms in Node
(14.5x); the repository's npm-compat spike measured about 6x on mixed work.
Most Electron main processes are glue (IPC, files, windows), and crypto, zlib
and SQLite are Rust compiled to WASM, not JavaScript. Apps that compute in
main (sync engines, language servers, bundlers) will feel it.

Ways out, in order of cost: move hot built-ins to host calls; SpiderMonkey
with its portable baseline interpreter and ahead-of-time specialisation
instead of QuickJS; the real Node core (libuv and Node's own library, as
Edge.js does) on the Linux ABI after M3, which should also lift Node
compatibility (Edge.js reports 3,592 of 3,626 Node tests, on Wasmer). The kill
criteria in section 8 say when to switch.

One spike artifact to record: IPC over the spike's HTTP transport showed a
43 ms plateau. It is TCP delayed ACK on a reused connection (1.0-1.9 ms on
fresh connections; first byte after 0.35 ms, last after 41 ms), not the
engine. The product path uses the worker wire, not TCP.

### 7.7 Packaging

`chariox program pack <app directory or asar>` writes a Chariox package: the
app's files under `/app`, a manifest (name, icons, version from
`package.json`; capabilities), the addon map applied, and a report of every
Electron member and addon the app uses that Chariox does not have yet. Nothing
in the app's source is edited. An agent can run the tool end to end (P2).

## 8. Milestones

Lane-week = one agent lane for one week, review included. Estimates are
ranges, to be corrected after each milestone's first spike. Every milestone
ends with its validation cells (section 10) accepted by Miguel.

### M1. Electron (30-40 lane-weeks, plus 8-10 optional)

| Part | Scope | Lane-weeks |
|---|---|---|
| M1a | Program world: `run`, lifetime, limits, logs, VFS layout, sockets under network policy with private loopback, ahead-of-time compile at install. Chariox Node runtime (wasm-rquickjs adopted), `asar`, workers and `fork` as instances. Pack tool. Agent tools to start, stop and read a program | 8-10 |
| M1b | `electron` tier 1 (the 197 members of the four simplest apps). Windows on private kernel browser contexts: preload, IPC, schemes, menus and dialogs as data, clipboard, notifications. Mirror view for program windows in Web and TUI clients. Agent cells | 12-16 |
| M1c | Tier 2 (to 429 members): `sendSync`, `@electron/remote`, Node integration bridge, `webRequest`, sessions, child windows and `<webview>`, printing, downloads, `utilityProcess`, tray, addon map v1 | 10-14 |
| M1d (optional, Q1) | Client-rendered fast path | 8-10 |

Apps: M1b minimal-repro, drawio-desktop, Simplenote, Standard Notes. M1c
MarkText, Zettlr, Bruno, Mattermost. Joplin, Insomnia, Beekeeper Studio and
Tabby need compiled addons, ptys or database sockets and are M3 cells. Logseq,
Element, Signal and VS Code get their census in M1a and are scheduled then.

Spikes first: S7, S8, S9 (section 9).

Risks: JavaScript speed (7.6); Node compatibility of real main bundles (only
samples ran so far); mirror latency for typing over the relay; memory per
window on the kernel host; apps that detect Electron internals.

Kill criteria:

- After 3 lane-weeks of runtime fixes, fewer than 4 of the 5 simplest apps'
  real main bundles load unpatched → stop extending QuickJS; build the Node
  runtime from the real Node core on the Linux ABI (pulls the thread and
  process part of M3 forward, about 10-12 lane-weeks).
- Main start to first window over 3x Electron's on the same host, or over 5 s,
  for a tier-1 app → engine work before M1c.
- Remote typing echo in Simplenote misses P95 ≤ RTT + 60 ms after the browser
  latency work → ship kernel-rendered windows for local and low-RTT use and
  pull M1d forward for remote.

### M2. Display interface v1 (14-18)

Scope: `chariox:display` (surfaces, scene, pixels, input, text input,
clipboard, drag and drop, dialogs, menus, output), the accessibility tree
path, the program view for Web and TUI clients (CanvasKit, semantic DOM,
hidden input), `chariox.program` tools, kernel-side rasteriser for agent
screenshots. Toolkits: egui, Slint, iced, Dear ImGui as component programs.
Can start after M1a, in parallel with M1b.

Spikes: S10, S11. Risks: scene fidelity for text (fonts, sub-pixel
positions); CanvasKit download size and start time on phones; IME on mobile
browsers. Kill criteria: if client rasterisation of the reference scenes
cannot hold 55 fps on the reference phone, or text differs visibly from the
program's own raster, the scene tier is limited to desktop clients and pixels
with lossless tiles becomes the default; if tiles then miss the latency gate
remotely, stop and re-plan the display tier before M4.

### M3. Linux-ABI programs (20-28)

Scope: the process layer (threads, futexes, processes, `exec`, pipes,
`epoll`, signals, AF_UNIX, ptys, `mmap` emulation, `/proc`, `/dev`); the SDK
(clang, musl port, libc++, Rust std, post-link tool, CMake, Meson, autoconf
and Cargo toolchain files); base runtime pack (dash, bash, coreutils, git,
curl, OpenSSH client, CPython, ripgrep); terminal programs on the Chariox
terminal surface; fork flavour; the second Node runtime if M1 called for it.
Can run in parallel with M1b and M2.

Spikes: S12, S13, S14, S15. Risks: soundness of host access to shared memory;
long tail of system calls; build systems that probe the host; fork cost.
Kill criteria: if an audited process layer cannot be made sound on stock
Wasmtime shared memory, fall back to one OS process per WASM thread group with
OS shared memory, or patch Wasmtime, and say which in a re-plan; if the Linux
Test Project subset for the calls we claim stays under 95% after 6 lane-weeks,
stop adding programs and fix the layer first.

### M4. Linux desktop endpoints (18-26)

Scope: Wayland endpoint (shm buffers, xdg-shell, seat, text-input,
data-device, cursor shape, fractional scale), D-Bus session bus, portals
(file chooser, open URI, notification, settings, secret, print), AT-SPI to
AccessKit, fonts and themes runtime pack. Toolkits unchanged in pixels: SDL 3,
GTK 4, GTK 3, Qt 6, wxWidgets. Needs M2 and M3.

Spikes: S16, S17, S18. Risks: toolkit build systems on a new target; pixel
bandwidth on remote links; AT-SPI coverage. Kill criteria: if GTK 4 and Qt 6
demos do not both run unchanged after 8 lane-weeks, drop the "unchanged"
route for the failing toolkit and go straight to its M5 backend.

### M5. Scene backends for GTK 4 and Qt (16-24)

Scope: GSK renderer and Qt platform plugin with recording paint engine; Qt
Quick through the software adaptation; direct AccessKit from both; native
menus and dialogs through the display interface. Needs M4.

Spikes: the GSK and QPainter recordings of S10, and S17. Risks: scene
coverage of rare drawing calls (fall back to an image for that node); Qt
private API churn. Kill criteria: if a backend cannot render the
toolkit's own demo with under 2% differing pixels against its reference
renderer, that toolkit stays on pixels and the backend is stopped.

### M6. GPU, audio, media (20-30)

Scope: `chariox:gpu` (WebGPU and GLES shapes) kernel-side, then client-side
replay; EGL in the SDK; `chariox:audio` with the sysroot libraries; video
playback regions; microphone and camera with permission prompts. Needs M2;
M3 for most programs.

Spikes: S19, S20. Risks: kernel hosts without a GPU; replay state for device
switches; audio latency over the relay. Kill criteria: if client-side replay
cannot switch devices without a visible reload, it ships for single-client
sessions only and kernel-side rendering stays the multi-client path; if audio
misses RTT + 100 ms, it ships as playback-only without the sync promise.

### M7. More toolkits and languages (50-80)

Scope, each its own sub-milestone with its own acceptance: Flutter (12-20),
.NET and Avalonia (8-12), Go on the Linux ABI (8-12), Java with Swing and
JavaFX (10-16), Compose (6-10), Tk and Xlib programs through Xwayland (6-10).
Order by demand (answer Q10).

Spikes: S21-S24. Kill criteria per sub-milestone: if its spike does not
produce a running window in 2 lane-weeks, that toolkit is parked with the
reason written down.

### Order and dependencies

```
M1a ──> M1b ──> M1c ──> (M1d)
  ├───> M2 ──────┐
  └───> M3 ──────┴──> M4 ──> M5
                 ├──> M6
                 └──> M7 (after M4 for toolkit parts)
```

With three lanes: the first Electron apps (M1b) after about 8-10 calendar
weeks; everything through M5 in about 9-12 months; M6 and M7 after that. These
are projections from the estimates above, not measurements.

## 9. Spikes

### 9.1 Run during this plan

Scratch was under `/var/tmp/anyprogram` and is deleted. Sources, outputs and a
`RESULT.md` per spike are under `/root/.codex/evidence/anyprogram/`. Host:
x86_64 Linux, stock `wasmtime` 50.0.2 CLI or a host on the `wasmtime` 50.0.0
crate, wasi-sdk 34, rustc 1.99.1.

| # | Question | Result |
|---|---|---|
| S1 | Can a pthread program use several cores on stock Wasmtime 50, without a fork? | **Yes.** `-S threads` is gone (wasi-threads removed in 47); wasip2 does not link `-pthread`; wasip3 compiles but `pthread_create` fails. A wasip1-threads module on our own 67-line host (shared memory on, one instance per thread) gives 3.69x, 4.02x, 3.63x on 4 threads; native 3.96x |
| S2 | Do plain sockets work, and are they closed by default? | **Yes, both.** No grant: `socket: Permission denied`. With a grant: TCP listen, accept, echo; name lookup; outbound HTTP 200; UDP. `SOCK_RAW` is not declared by wasi-libc; AF_UNIX is "address family not supported" |
| S3 | Does an unmodified C++ immediate-mode toolkit build and run, and how big is its output? | **Yes, with one define** (ImGui pulls `<sys/wait.h>` for shell helpers). 1.7 MB component; 0.147-0.159 ms per frame in Wasmtime against 0.067-0.070 native (2.1-2.3x). Draw data 157 kB per frame raw; as list deltas 1,038 B idle, 23 B hover, 82.5 kB while typing and scrolling |
| S4 | For a Rust toolkit: scene size, mesh size, tree size, CPU? | egui 0.33.3 on wasip2, 3.3 MB. Scene 17.3 kB per full frame, delta 12-77 B (11 kB while typing and scrolling); meshes 228-235 kB; AccessKit 332 nodes, deltas 0-1.2 kB (217 kB while scrolling, absolute bounds); 0.32-0.72 ms per frame, 0.9-1.7x native. winit 0.30 does not build for WASI |
| S5a | Which Electron APIs do real apps use? | Static census of 12 apps: 494 members; the 197 of the four simplest cover 61-71% of each later app; native addons in 9, spawned programs in 9, `sendSync` in 4, `@electron/remote` in 4, Node integration in 4; Chromium-control features in almost all (7.3) |
| S5b | Does an unchanged Electron main process run in Wasmtime with `electron` as a plain module? | **Yes.** Three official Electron 45 samples pass with a headless Chrome as the window: IPC both ways, menu as data, dialog as a request. 7.0 MB component, ready in 8-14 ms, 45-59 MB peak. `fib(27)` 29 ms against 2 ms in Node. A 43 ms IPC plateau was TCP delayed ACK in the spike's transport, not the engine |
| S6 | Can signals be delivered into a running guest? | **By guest polling, yes; by the engine, no.** Wasmtime 50's epoch callback gets no mutable store, so the host cannot call a handler from it without a patch. Polling a pending word: handler ran 0.2-1.2 ms after `raise` in a loop that never calls the host; cost on a real loop -2.1% to +1.7%; 8x only when a source-level poll blocks folding of a trivial loop |

What these settle: no Wasmtime fork is needed for threads, sockets or signals;
the scene tier is worth building; the Electron mapping is sound for the main
process. What they do not settle: real app bundles, windows on the kernel
browser, the Linux ABI itself, and every toolkit build.

### 9.2 To run, first thing in each milestone

| # | For | Exact question |
|---|---|---|
| S7 | M1 | With drawio-desktop's window as a kernel browser tab in a private context, preload injected and IPC over protocol bindings: does the app open, edit and save a file unchanged? Input echo P50 and P95 locally and at RTT 80 ms? |
| S8 | M1 | Load the real main bundle of each census app in the Chariox Node runtime with a recording `electron` module: how many reach `whenReady` and the first `BrowserWindow`? Which Node APIs fail? Start time against Electron? |
| S9 | M1 | `sendSync` and a `require('fs')` proxy from a kernel browser page over a blocking request: per-call time? Do MarkText's and Zettlr's sync calls work unchanged? |
| S10 | M2 | Record scenes from egui, GSK and QPainter programs; replay them in CanvasKit, Canvas 2D and a WebGPU vector renderer on Chrome, Safari (iOS) and Firefox: frames per second, pixel difference from the program's own raster, download size, start time? |
| S11 | M2 | A hidden input at the caret with composition (Japanese, Chinese, emoji, dead keys) on Chrome, Safari iOS and Firefox against an egui text field: is every commit and pre-edit correct? Does the phone keyboard open and stay? |
| S12 | M3 | Our musl port on the system-call imports, on the thread host: do dash, coreutils and CPython build without source changes and pass their own tests? How many distinct system calls do they need? Linux Test Project pass rate for those? |
| S13 | M3 | Does `vfork`+`exec` work uninstrumented on exception-based `setjmp` (kill criterion in 4.2); `fork` by Asyncify against fork-only instrumentation, on bash and CPython `multiprocessing`: size, speed, fork time at 100 MB of memory? |
| S14 | M3 | `dlopen` of a side module in the instance-per-thread model: can table and memory growth be replayed into running threads? Load CPython's `_ssl` and a Qt plugin |
| S15 | M3 | Audit: can every host access to shared guest memory in the process layer be an atomic-safe copy, with a check that fails the build otherwise? Any Wasmtime API that forces a plain borrow? |
| S16 | M4 | SDL 3 `testsprite` and `gtk4-demo` built with the SDK against a minimal Wayland endpoint with shm buffers: do they draw and take input? Bytes per second as lossless tiles while typing and scrolling? |
| S17 | M4 | qtbase built statically for the Linux ABI with the `offscreen` and `wayland` plugins: does a Widgets example paint? Module size, compile time at install, start time? |
| S18 | M4 | AT-SPI trees of gtk3-demo and a Qt Widgets example converted to AccessKit: node and action coverage against the toolkit's own inspector? Update rate under scrolling? |
| S19 | M6 | A wgpu program through `wasi-gfx-runtime` on Wasmtime 50, on a host with and without a GPU: frames per second? Command bytes per frame? Does a recorded stream replay in a browser's WebGPU? |
| S20 | M6 | PCM → Opus → browser over the relay path at RTT 80 ms: end-to-end latency and drift over 10 minutes? |
| S21 | M7 | A Flutter counter compiled with dart2wasm, run outside a browser: which browser APIs does the engine's web layer import? Count and group them |
| S22 | M7 | An Avalonia headless program through NativeAOT-LLVM to WASI: does it build and run? Threads? Size? |
| S23 | M7 | How many runtime and `syscall` files does a Go port to the Linux ABI touch, measured from the upstream wasip1 port's diff? |
| S24 | M7 | OpenJDK Zero built with the SDK: do `java -version` and a headless Swing paint run? Interpreter speed against HotSpot? |

## 10. Validation cells

Miguel accepted these rows on 2026-11-17 as the scenarios of each milestone. Programs
are real open-source programs, built from unchanged upstream source (Electron
apps: unchanged release files); versions are pinned when the milestone starts.

Conditions for every row unless it says otherwise:

- **Clients:** Web; local TUI (default browser); remote TUI (default browser).
  iOS rows run when the iOS App view track has landed, first in a WKWebView.
- **Local:** client on the kernel host. **Remote:** hosted relay WSS, about
  8 Mbit/s client uplink, measured RTT ≥ 60 ms recorded next to every P95.
- **Agent rows:** a real agent, no client attached, no computer use, actor
  receipts checked.
- Every row is repeated on a Linux x86_64 and a Linux arm64 kernel host; rows
  marked (mac) also on a macOS kernel host, to prove one implementation.

### M1 Electron

| ID | What Miguel or a validator does | Pass condition |
|---|---|---|
| AP1-01 | An agent runs unchanged Node programs as programs: `http-server` on a folder, `prettier --check` on a repository, `tsc` on a small project | Correct output and exit codes; the served page opens in the kernel browser by an explicit action only; start and run times recorded against Node |
| AP1-02 | Pack the drawio-desktop release with one command, install, open in Web. Draw, save, reopen, export PNG to the device | No source edit. Saved file is in the program's data; export arrives through the save dialog; menus work from Chariox chrome |
| AP1-03 | With AP1-02 open, open the same program in the remote TUI's browser and take over. Type a label | Same document on both. Takeover is explicit. Local input → paint P95 ≤ 50 ms; remote echo P95 ≤ RTT + 60 ms |
| AP1-04 | Simplenote: install shows the network grant; sign in; edit a note on two clients | Sync with the real service works; typing meets the gates; without the grant the app shows its own offline error |
| AP1-05 | Standard Notes: sign in, quit, start again | Session restored from the Vault-sealed store; the stored secret appears in no agent-visible output, log or history |
| AP1-06 | File → Open in any app, from the menu on one client | The Chariox file dialog appears on every client; answering on one closes it everywhere; the app receives a granted path |
| AP1-07 | Agent, no client: start drawio, add three labelled boxes and two arrows with DOM and menu actions, save, export SVG. Then a person opens the program | Exported SVG has the elements; the person sees the same document; no screenshot-and-pointer steps in the receipts |
| AP1-08 | Agent triggers Open and answers the dialog with a file it holds a grant for; then with one it does not | First loads; second is refused with a clear reason |
| AP1-09 | M1c apps: MarkText (edit, folder search), Zettlr (export with pandoc), Bruno (send requests to a real API, script a test), Mattermost (join a real server, receive a message) | Each named flow works unchanged; Mattermost notification reaches every client; every unsupported member the app touched is in the pack report, none fails silently |
| AP1-10 | Isolation: read the person's browser cookies from a program window; run two programs that listen on the same loopback port; open a socket without a grant | Nothing shared with the person's browsing; both listen; the socket is refused with an error naming the missing grant |
| AP1-11 | Close every client for an hour, reopen. Restart the kernel, reopen. Update the package while it runs | Same window state after the hour; after restart a cold start that restores from the app's own files; update asks before replacing a running program |
| AP1-12 | Pack an app that needs an unmapped native addon | The pack report names the addon and the route (tier 2 or impossible) before install |
| AP1-13 | Record, for every app: start to first window, memory, and one heavy main-process task, against Electron on the same host | Table published with the acceptance; kill criteria of M1 evaluated on it |
| AP1-14 (iOS) | Open AP1-02 and AP1-04 on the iOS client | Same document; typing with the on-screen keyboard meets the remote gate |

### M2 Display interface

Programs: `egui_demo_app` and one more eframe program; Slint `printerdemo`
and `slide_puzzle`; Halloy (iced IRC client, real TCP and TLS); Dear ImGui
demo with ImPlot.

| ID | What Miguel or a validator does | Pass condition |
|---|---|---|
| AP2-01 | Open each program on each client, use every widget class | Sharp at scale 1, 2 and 3; visually equal to the program's own raster (pixel difference threshold set by S10) |
| AP2-02 | Type Japanese with an IME on desktop; type on a phone browser's keyboard; dead keys | Pre-edit shown at the caret; commits correct; keyboard opens on focus and closes on blur |
| AP2-03 | Copy and paste both ways; drop a file from the device onto the window; save a file to the device | Text and images cross; the dropped file arrives as a path under `/mnt`; save uses the Chariox dialog |
| AP2-04 | Latency and bandwidth run: idle, hover, type, scroll, animation | Local P95 ≤ 50 ms and ≥ 55 fps; remote echo P95 ≤ RTT + 60 ms; ≤ 10 kbit/s idle and ≤ 2 Mbit/s P95 interacting |
| AP2-05 | Use the window with the client's screen reader (VoiceOver, NVDA) | Roles, names and values are read; focus follows; actions work |
| AP2-06 | Agent, no client: snapshot, find a field, set its value, click a button, wait for the result, take a screenshot; fill a password by Vault handle | Works on every toolkit with a tree; the filled value is masked in snapshot and screenshot; ImGui is driven by declared actions |
| AP2-07 | Laptop → phone → laptop while the program runs | The window resizes to each device within RTT + 250 ms; state kept |
| AP2-08 | Drop the connection for 30 s; reconnect | Last frame stays; live again within RTT + 500 ms; no lost or repeated input |
| AP2-09 (mac) | Halloy on a macOS kernel host | Same behaviour as on Linux |

### M3 Linux-ABI programs

| ID | What Miguel or a validator does | Pass condition |
|---|---|---|
| AP3-01 | In the Chariox terminal on each client: bash, vim, htop, tmux, git clone over HTTPS, a Python REPL using `threading` and `subprocess` | All built from unchanged upstream source with the SDK; interactive use correct; terminal echo meets the gates |
| AP3-02 | `xz -T4` and ripgrep on a large tree, 1 thread against 4 | Speed-up ≥ 3x on 4 cores; CPU time within 2.5x of native |
| AP3-03 | Shell pipelines, `make -j4` of a small project with WASM tools, `pip install` of a pure-Python wheel, Python `multiprocessing` (fork flavour) | Correct results and exit codes; `exec` of a native binary is refused with a clear error |
| AP3-04 | Servers: redis-server with redis-cli (background save uses fork), nginx serving a folder, `sqlite3` | Work; reachable only inside the program or through an explicit open; data persists across restarts |
| AP3-05 | Ctrl-C on a busy Python loop; `timeout 1 sleep 10`; stop a program that saves on `SIGTERM` | Interrupt within 50 ms; timeout fires; the program saves before it exits |
| AP3-06 | OpenSSH client to a real host and `curl` over HTTPS with a grant; `ping` | Work with the grant, refused without; ping works through unprivileged ICMP |
| AP3-07 | Electron tail: Joplin (SQLite, sync), Insomnia (compiled `node-libcurl`), Beekeeper Studio (connect to a real PostgreSQL), Tabby (local shell on a Chariox pty, SSH) | Named flows work from unchanged release files plus recompiled addons |
| AP3-08 | Agent, no client: run `git log`, a Python script and a build as actions | Output, exit code and duration returned; no terminal scraping |
| AP3-09 (mac) | AP3-01 and AP3-03 on a macOS kernel host | Identical results from the same package |
| AP3-10 | `vfork` proof: Python `subprocess` (its `vfork` path), `posix_spawn` from a C program, and `git` starting `ssh` and a hook, all built without the fork flavour; then the same C program calling `fork` | The spawns run to `exec` with correct exit codes and no stack copy (binary size and spawn time equal to a build without fork support); the plain `fork` call fails with `ENOSYS` and the message that names the flavour. Kill criterion: if any spawn needs the fork flavour, 4.2 and AP3-03 change to say so before M3 work continues |

### M4 Linux desktop endpoints

Programs: GNOME Text Editor and GNOME Calculator (GTK 4, libadwaita); Geany
(GTK 3); KeePassXC, qBittorrent and FeatherPad (Qt 6); ScummVM with a
freeware game, video and input only (SDL); FileZilla (wxWidgets).

| ID | What Miguel or a validator does | Pass condition |
|---|---|---|
| AP4-01 | Build each program from unchanged upstream source with the SDK; open on each client | No source patch (build flags only); windows, popups, menus, tooltips, dialogs correct |
| AP4-02 | Open, edit and save files through each program's file dialog; print to PDF | The portal dialog is the Chariox dialog on every client; file lands where chosen |
| AP4-03 | Clipboard, drag and drop, IME, cursor shapes, dark and light theme | Same pass conditions as AP2-02 and AP2-03; theme follows the active client |
| AP4-04 | qBittorrent downloads a legal torrent; FileZilla connects to a real SFTP server | Sockets under the grant; threads scale; data persists |
| AP4-05 | Agent, no client: compute in Calculator; create an entry in KeePassXC with the master password filled by Vault handle; find text in Geany | Done through the tree and derived menu actions; the password is masked in everything the model sees |
| AP4-06 | Latency and bandwidth, pixels tier | Local P95 ≤ 50 ms; remote echo P95 ≤ RTT + 60 ms; ≤ 8 Mbit/s P95 interacting; a miss is reported as a miss |
| AP4-07 | Screen reader on the client for one GTK and one Qt program | As AP2-05 |

### M5 Scene backends

| ID | What Miguel or a validator does | Pass condition |
|---|---|---|
| AP5-01 | The M4 GTK 4 and Qt programs, same source, scene backends on | Sharp at scale 1, 2, 3; under 2% differing pixels against the toolkit's reference renderer on its demo |
| AP5-02 | The AP4-06 run again | ≤ 10 kbit/s idle and ≤ 2 Mbit/s P95 interacting; gates met remotely |
| AP5-03 | A Qt Quick program (a Qt Quick Controls gallery) | Runs through the software adaptation; animation ≥ 55 fps locally |
| AP5-04 | Agent cells of AP4-05 with the direct AccessKit trees | Same results; tree update P95 ≤ 100 ms |

### M6 GPU, audio, media

Programs: Chocolate Doom and ScummVM with sound; SuperTuxKart (GLES); a Bevy
example and an egui program on wgpu; VLC playing a local file; Audacity
(record, edit, play); KiCad 3D viewer.

| ID | What Miguel or a validator does | Pass condition |
|---|---|---|
| AP6-01 | Play each game for five minutes on each client, kernel host with a GPU and without | ≥ 30 fps at 720p remotely on the kernel-side path; input meets the gate or the miss is recorded; no desync |
| AP6-02 | VLC: play a 1080p file with sound | Audio and video within 80 ms of each other; audio latency ≤ RTT + 100 ms |
| AP6-03 | Audacity: record from the client's microphone, play back | The Chariox permission prompt appears once on every client; recording is clean |
| AP6-04 | Client-side replay: SuperTuxKart on a kernel host without a GPU; then switch device | ≥ 55 fps on the client; the switch resumes without a restart of the program |
| AP6-05 | Agent, no client: export audio from Audacity by menu actions; screenshot the KiCad 3D view | Works with kernel-side rendering |

### M7 More toolkits and languages

One row set per sub-milestone, each with the M2 cells AP2-01 to AP2-08 as its
pass conditions, on these programs: Flutter Gallery and LocalSend (Flutter);
AvaloniaILSpy (Avalonia); JabRef (JavaFX) and jEdit (Swing); lazygit and
Syncthing (Go; Syncthing's web UI opens in the kernel browser); the KotlinConf
app (Compose); gitk and IDLE (Tk).

## 11. What we fork or carry, and what it costs

| Thing | Fork? | What we carry | Why | Yearly cost (estimate) |
|---|---|---|---|---|
| Wasmtime | **No** | Nothing. The process layer uses tier-2 features (threads, shared memory), so each engine bump runs the thread and process conformance suite | Threads, signals and processes all have routes on the stock engine (S1, S6). A fork of a monthly-released engine is the most expensive thing we could own | 2-3 lane-weeks (bumps) |
| WASI | No | New `chariox:*` WIT packages. `chariox:gpu` follows `wasi:webgpu` so we can converge | WASI has no process, signal, display or audio interface and none is proposed | Included in milestones |
| musl | **Yes, a port** | The `wasm32` Linux port from WALI (MIT) plus ours: system calls as imports, `vfork` on `setjmp`, signal poll at system-call return | No libc targets a virtual Linux on WASM. Upstreaming was proposed in April 2026 without reply | 1-2 lane-weeks (rebase per musl release) |
| LLVM, clang | No | A Chariox SDK build: sysroot, compiler-rt, libc++ with exceptions and threads; toolchain files for CMake, Meson, autoconf, Cargo | The triple is upstream since LLVM 22 | 2 lane-weeks |
| Post-link tool | Ours | Signal polls, the fork flavour, checks. Uses Binaryen for Asyncify | No compiler change needed | 1 lane-week |
| Rust | No | A prebuilt `std` for the tier-3 `wasm32-wali-linux-musl` target, per pinned compiler | Tier 3 ships no `std` | 1 lane-week |
| wasm-rquickjs | Dependency first; fork if upstream stalls | Electron-driven fixes, sent upstream | It is the only Node-shaped runtime that is already a WASI 0.3 component | 4-6 lane-weeks |
| Wayland, D-Bus, AT-SPI | No | Our own endpoints in the worker (existing Rust protocol crates) | Toolkits speak these already | 2 lane-weeks |
| GTK | M5 only: **patch set** | One GSK renderer and its GDK hook; GTK does not load renderers from outside its tree | Scene quality. M4 needs no patch | 1-2 lane-weeks (two GTK releases) |
| Qt | No | An out-of-tree platform plugin on Qt's private platform API, pinned to Qt LTS; build files for the target | Plugins are Qt's own extension point | 2-3 lane-weeks |
| SDL, wxWidgets, Dear ImGui, egui, Slint | No | Integration crates or backends outside their trees; none for SDL and wxWidgets | - | 1 lane-week together |
| winit | Patch set until its backends can live out of tree | A Chariox backend | 0.30 has in-tree backends only | 1 lane-week |
| Flutter engine | **Yes** (M7) | The web layer with a Chariox renderer backend | dart2wasm output is tied to browser APIs | 4-6 lane-weeks |
| Go toolchain | **Yes** (M7) | A port to the Linux ABI | Upstream has wasip1 only | 2-4 lane-weeks (two Go releases) |
| OpenJDK | Patch set (M7) | Zero interpreter build for the target, AWT on Wayland | No JVM targets WASI | 2-3 lane-weeks |

Standing cost once everything exists: about 25-35 lane-weeks a year, of which
M7 forks are a third. Through M6 it is about 18-24.

Licences: nothing GPL goes into the runtime or the SDK (so no Kandelo code).
Programs keep their own licences. A statically linked LGPL library (Qt, GTK)
obliges the packager to ship relinkable objects; the pack tool does that by
keeping the object archive in the package (answer Q10).

## 12. What stays impossible, and why

"No limitations" holds for everything a program is, given its source. These
remain, each for a stated reason:

1. **Code that exists only as native binaries** (closed libraries, prebuilt
   addons, drivers). WASM needs source or a WASM build.
2. **JIT compilers that emit machine code** (V8's JIT, LuaJIT, HotSpot, .NET's
   JIT, PyPy). They run as interpreters or ahead-of-time builds, slower. A JIT
   that emits WASM can be supported through 4.7 later.
3. **A web engine itself.** Chromium, WebKit and Gecko do not run in WASM. Web
   UI runs in the kernel browser or the client's engine.
4. **More than 4 GiB per process** until memory64 is enabled.
5. **`fork` without `exec`** needs the fork build flavour, and its memory copy
   is real.
6. **Live shared memory between processes** (`MAP_SHARED` across `fork`,
   System V shared memory) is coherent only at sync points for now. PostgreSQL
   is the known program this stops.
7. **Raw frames on the host's real network** need a host privilege. Raw
   sockets work inside the program's virtual network.
8. **Hardware:** USB, serial, Bluetooth, hardware video codecs, DRM-protected
   media. Only through a Chariox interface if one is added later.
9. **Speed:** compiled code about 1-2.5x native CPU; interpreted languages
   without their JIT 6-15x slower than with it.
10. **Programs without an accessibility tree** give agents only actions and
    the scoped pointer fallback.
11. **Fault handlers that resume** (`SIGSEGV` tricks used by some garbage
    collectors and JITs) do not work; those runtimes need their portable mode.

"Mostly as is" means: built from unchanged source with the Chariox SDK and
build flags. Electron apps are not rebuilt at all.

## 13. Answers (accepted 2026-11-17)

Miguel accepted all ten recommended answers. The plan above is built on them.

- **Q1.** Electron windows render in the kernel browser, mirrored to every client; a client-rendered fast path is optional and later (M1d).
- **Q2.** The window resizes to the device that takes over input; passive clients see it scaled.
- **Q3.** A program lives until it exits or its last window is closed; a kernel restart is a cold start; no snapshots.
- **Q4.** The packager declares the network; "any host" is allowed and shown as one consent line at install; loopback is always private.
- **Q5.** Programs are told they run on Linux, through a Chariox-implemented Linux ABI.
- **Q6.** This plan replaces the 2026-10-22 decision against wrapped binaries; that section is deleted.
- **Q7.** Start with QuickJS for the Electron main process; switch by the M1 kill criteria.
- **Q8.** GPU programs render in software on the kernel first; client-side replay comes second.
- **Q9.** An agent may use screenshot plus pointer in one program's window only as a marked fallback when the program has no tree, never when a tree exists.
- **Q10.** The milestone order stays as proposed; Chariox packages the validation set as references, the SDK and pack tool are for everyone, and LGPL objects ship in the package.

The validation cells in section 10 are the accepted scenarios.

## 14. References

- This repository: `spikes/wasm/APPS_V2_PLAN.md`, `spikes/wasm/APPS_V2_SHAPE.md`,
  `packages/app-wit/wit/app.wit`,
  `packages/app-wit/wit/admission.wit`, `packages/app-wasm-worker`,
  `docs/APP_VIEW_HOST_AV1.md`, `docs/APP_VIEW_DELIVERY_AV2.md`,
  `docs/MULTIDOMAIN_DOM_MIRRORING.md`, `docs/MULTIDOMAIN_KERNEL_BROWSER_DISPLAY.md`,
  `docs/BROWSER_LATENCY_GAP_PLAN.md`, `docs/APP_EXTERNAL_FILES.md`,
  `docs/CHARIOX_DISTRIBUTION_PLAN.md`.
- Evidence, outside the repository, under `/root/.codex/evidence/anyprogram/`:
  `s1-threads`, `s2-sockets`, `s3-imgui`, `s4-egui`, `s5a-electron-census`,
  `s5b-electron`, `s6-signals`, `research/linux-on-wasm.md`.
- Outside projects named here, as found on 2026-11-17: Wasmtime 50.0.2; WASI
  0.3.0; wasi-sdk 34; WALI (`Wasm-Thin-Kernel-Interfaces/WALI`) and its paper
  (arXiv 2312.03858); lind-wasm; Kandelo; WASIX and Edge.js; wasm-rquickjs
  0.5.3; `wasi-gfx-runtime` 0.5.2; AccessKit; GTK Broadway; Flutter web;
  Electron 45.3.3.
