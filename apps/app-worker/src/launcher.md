# Native App worker launch contract, ABI 1

This is the internal supervisor/launcher contract. It is not an App API or a
user-facing command. The launcher has no CLI arguments, profile override,
executable override, environment override, or probe switch. The existing SDK
channel remains **FD 3**; the startup control channel is **FD 4**.

The native loader and macOS libc probe can be compiled independently of Node.
The local probe evidence is unsigned and does not establish signed-runtime JIT,
Node compatibility, Linux containment, resource overshoot, or release readiness.
Those are required implementation and validation work, not per-user setup.

## Supervisor obligations before exec

1. Verify and hold the signed launcher/runtime bundle and its trusted SDK
   bootstrap. Pin the installation's exact verified package and generation.
   Runtime and package contents and their parent directories must remain
   immutable to Apps for the worker lifetime. Do not derive bootstrap bytes,
   native library choices, roots, or Node arguments from App/client requests.
2. Provision private quota-enforced data/tmp roots and admit the worker against
   per-installation and aggregate resource budgets. Set up the platform's
   memory/CPU/thread accounting and termination monitor before continuation.
3. Use an explicit clean environment in `execve`/`posix_spawn`, including no
   `LD_*`, `DYLD_*`, `NODE_*`, provider credentials, or inherited agent state.
   Filtering inside `main` cannot undo dynamic-loader injection before `main`.
   Create a new session without a controlling terminal. Pass only the intended
   descriptors below; the launcher additionally closes ambient descriptors.
4. On Linux, the installer-provisioned pinned **bundled** bubblewrap establishes
   user, mount, PID, IPC and network namespaces and drops every capability. Use
   a private read-only root containing only the allowed mounts, not a bind of
   host `/`. Attach the worker to its delegated cgroup before continuation.
   The provider sandbox's existing bubblewrap argument list is not this policy.
   All setup is mandatory; setup failure cannot select an unconfined execution
   path. Complete and test this supervisor/provisioning integration separately.

The native launcher validates canonical absolute roots, ownership and absence of
group/other-writable ancestors, disjoint paths/inodes, inherited streams, bounds,
and Linux mount/capability invariants. These checks complement the supervisor's
held verified objects; they do not replace artifact signatures or independently
attest that the supervisor created every namespace or cgroup correctly.

## Descriptors and environment

| FD | Contract |
| --- | --- |
| 0 | Replaced with read-only `/dev/null` before reading the launch record |
| 1, 2 | Supervisor-owned bounded log pipes/streams; never terminals |
| 3 | Private connected AF_UNIX stream for the existing SDK protocol |
| 4 | Separate private connected AF_UNIX startup stream, closed before `dlopen` |
| 5+ | Closed before parsing the record; Linux uses `close_range`, macOS enumerates its process descriptors |

Both channels are created by the supervisor for one exact worker; channel
ownership binds authority. There is no bearer token in argv/environment. The
SDK still authenticates every request against the channel's installation and
generation. App JavaScript cannot establish identity by changing environment
values. The launcher keeps only generated `HOME`, `TMPDIR`, `LANG`, `TZ`,
`UV_THREADPOOL_SIZE=4`, and `CHARIOX_APP_{GENERATION,INSTALLATION,RELEASE_DIGEST,
PACKAGE,DATA,TMP}`. These values are context, not credentials.

## Launch record

All integers are unsigned big endian, with no native structure padding. The
supervisor writes `CXAWL001` (8 ASCII bytes), then a u32 payload size, then that
exact payload. The payload limit is 280000 bytes. A string is a u32 byte length
followed by exactly that many UTF-8 bytes, without a terminating NUL.

| Order | Type | Field and bound |
| --- | --- | --- |
| 1 | u32 | `nofile`: 32..4096 |
| 2 | u32 | `cpuSeconds`: 1..86400 |
| 3 | u32 | `heapMiB`: 16..65536 |
| 4 | u32 | `v8Threads`: 1..4 |
| 5 | u64 | `maxFileBytes`: 1..2^40 |
| 6 | string | Generation: canonical positive decimal, no leading zero, at most i64::MAX |
| 7 | string | Installation ID: 1..128 ASCII letters, digits, `-`, `_`, `.` |
| 8 | string | Release digest: exactly 64 lowercase hexadecimal bytes |
| 9 | string | Canonical package path: 1..1024 bytes |
| 10 | string | Canonical data path: 1..1024 bytes |
| 11 | string | Canonical temporary path: 1..1024 bytes |
| 12 | string | Canonical verified runtime directory: 1..1024 bytes |
| 13 | string | Trusted bootstrap source: 1..262144 bytes |

Metadata strings reject NUL, ASCII controls and malformed UTF-8. Bootstrap
permits code whitespace but rejects NUL and malformed UTF-8. The bootstrap
length must exhaust the payload exactly. The supervisor converts the public
`sha256:...` digest representation only when building this internal record;
public metadata retains its existing representation.

Linux paths are fixed to `/app/package`, `/app/data`, `/app/tmp`, `/runtime`.
Package/data/tmp require `noexec,nodev,nosuid`; package additionally requires
read-only. Runtime requires `nodev,nosuid,ro` and permits executable library
mappings. Root `/` must be read-only. The supervisor provides `/dev/null` and
only the exact read-only system libraries required by the verified artifact's
dependency inventory. Do not expose host `/proc`, home, `/run`, devices, provider
IPC, or arbitrary system-library trees. macOS uses its canonical installation
paths and a default-deny Seatbelt profile, without claiming a virtual `/`.
This loader currently requires Linux kernel 5.9 or later for `close_range`;
the installer's complete supported-distribution floor remains to be selected.

## Ordering and continuation

The launcher starts one 15-second monotonic deadline covering record I/O,
preparation and continuation. Startup stream I/O is nonblocking and uses that
deadline. The supervisor additionally bounds the whole process startup; native
OS calls are not an independent hard wall-clock timer.

After descriptor filtering, path checks, cwd/data setup, environment replacement,
resource limits and successful OS-policy entry, the launcher writes:

```text
CXAWR001 + string(generation) + string(installationId) + string(releaseDigest)
```

The supervisor compares this exact identity with its admitted launch and writes
the eight-byte continuation `CXAWGO01`. A missing, malformed or late continuation
exits without loading the runtime. On success the launcher closes FD4, then
loads only `libchariox-app-runtime.dylib` or `libchariox-app-runtime.so` from the
verified runtime directory using `RTLD_NOW|RTLD_LOCAL`. Node and library static
initializers first execute at this point. The launcher checks the pinned Node
version `24.20.0`, invokes `runtime.h` ABI1 once and exits after it returns.

Node options are generated internally: permission mode, no add-ons, worker
threads, the admitted V8 heap setting, read access to all four roots, and write
access to data/tmp. There is no child-process, native-add-on or WASI permission.
Node's permissions are defense in depth; the native boundary must still contain
machine code that bypasses them. The trusted SDK bootstrap completes its own
supervised startup before importing the installation entry point.

## Platform policy and resource scope

The macOS profile denies by default; it permits private files, exact ancestor
metadata, self signals/process metadata and named CPU/OS sysctls. Read/executable
mapping support is restricted to the verified runtime, `/usr/lib`, the system
dyld directory and OS Cryptex paths. `file-map-executable` has an explicit deny
before the trusted library exceptions: relying on `(deny default)` while
allowing private `file-read*` failed the direct-mapping probe on macOS 14.8.9
([failed run 34169996404](https://github.com/charioxai/chariox/actions/runs/34169996404)).
The direct-mapping denial assertion remains mandatory; the probe also checks
compiled policy denial for the package and permission for the runtime. The
explicit policy correction passed on the same macOS 14.8.9 image in
[run 34170581264](https://github.com/charioxai/chariox/actions/runs/34170581264),
including the actual direct-mapping denial and all host-authority checks.
**The unsigned macOS probe observed that a read-only
file mapping can later become executable through `mprotect`. Seatbelt alone is
not evidence of a complete executable-memory policy.** The signed/hardened JIT
fixture must establish that stronger contract; the outer sandbox must contain
native/JIT code regardless of its origin. The profile grants no network
endpoints, process creation/execution, arbitrary
Mach service lookup, device writes, home/workspace access, or desktop access.
The signed release must keep hardened-runtime library validation and use the
V8 JIT entitlement, then test this policy against the pinned embedded Node.

Linux first checks prepared mount flags and zero capabilities, disables dumping,
sets `no_new_privs`, then installs an architecture-checked default-deny BPF
syscall policy. Threads require the thread-form `clone` flags; `clone3` returns
ENOSYS for libc fallback. New sockets, listeners, process creation/execution,
ptrace/process-memory access, memfd, namespace/mount changes, keyrings, BPF,
perf and io_uring are unavailable. The syscall allowlist is source to validate
on both production Linux architectures, not evidence of Node compatibility.
The mount boundary supplies filesystem policy; seccomp alone does not.
Descriptor copies and inotify watches operate only on reachable private paths
and inherited handles. Their memory/watch usage remains part of admitted cgroup
and filesystem resource budgets. The Linux native fixture includes private
copy/watch positives, escaped-watch denial, and limit query/mutation checks;
the actual pinned Node `fs.copyFile`/`fs.watch` cases still need execution.

`RLIMIT_CORE=0`, `RLIMIT_NOFILE`, `RLIMIT_CPU` and `RLIMIT_FSIZE` are hard native
limits. CPU here is a lifetime fallback, not a bandwidth allowance. FSIZE is a
per-file ceiling, not a storage quota. V8's old-space setting is not a total-RAM
limit. Total RAM, CPU bandwidth, threads, browser/broker work, aggregate host
reserves, installation disk quotas, cancellation and log/IPC bounds still need
the supervisor's provisioned mechanisms and measured release validation.

Exit statuses 100..109 are stable startup categories: invalid record, paths,
descriptors, native limits, sandbox, handshake, library load, runtime version,
environment, timeout. Errors contain only the category number, never a path,
bootstrap, payload or dynamic-loader diagnostic. Runtime ABI statuses 120..124
retain their existing meanings. Signals and resource deaths require the
supervisor's independent termination classification.

## Native probe

On macOS, run `node --test --test-concurrency=1
apps/app-worker/tests/native-launcher.test.mjs`. It compiles only the small C
launcher and test runtime with local clang, uses private scratch outside Git,
and deletes all binaries/state. No Node artifact is downloaded or built.

The same production launcher and policy load a **test-only unsigned** libc
runtime whose constructor checks confinement before the entry call. Tests cover
allowed private I/O, denied host/escaped/package-write/executable-map access,
network/fork/signal restrictions, environment/FD filtering, exact continuation,
record/path rejection and deadlines. A separately compiled deliberately weakened
policy must make the native probe fail. Neither probe artifact nor policy
selection is exposed to installed Apps. Native Linux namespace/cgroup fixtures,
signed macOS/JIT acceptance and the hostile JavaScript suite remain to implement.

Policy references: [Chromium Seatbelt entry points](https://github.com/chromium/chromium/blob/main/sandbox/mac/seatbelt.cc),
[Chromium macOS common policy](https://github.com/chromium/chromium/blob/main/sandbox/policy/mac/common.sb),
[Apple/WebKit explicit mapping policy](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/Resources/SandboxProfiles/ios/com.apple.WebKit.Networking.sb),
[Apple XNU mmap and mprotect checks](https://github.com/apple-oss-distributions/xnu/blob/xnu-10002.81.5/bsd/kern/kern_mman.c),
[Linux seccomp documentation](https://www.kernel.org/doc/html/latest/userspace-api/seccomp_filter.html),
[upstream bubblewrap](https://github.com/containers/bubblewrap).
