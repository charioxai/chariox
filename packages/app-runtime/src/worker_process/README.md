# Native worker lifecycle component

`WorkerProcess` implements the blocking process owner below kernel orchestration.
It writes native launch ABI 1, compares the complete returned installation,
generation and digest, verifies the retained resource domain, then sends Continue.
The existing generation-fenced `wire::Channel` uses FD3; startup control uses FD4.
Only the one SDK stream can be taken. A native Ready reply means startup ordering,
not independent proof of sandbox configuration or artifact trust.

`PreparedWorker` has no public constructor and no production factory yet. The
enrolled runtime verifier/platform provisioner must mint it after obtaining:

- Verified immutable launcher/runtime/bootstrap objects and the verified App
  release, with held descriptors **and** generation/pinning ownership preventing
  rename, replacement or cleanup of their paths throughout execution.
- Private data/tmp directory ownership and installation disk quotas.
- A retained per-installation and aggregate resource reservation, with memory,
  CPU bandwidth, process/thread, and domain termination mechanisms.
- On Linux, pinned bundled bubblewrap, the exact private read-only filesystem,
  noexec/nodev/nosuid App mounts, restricted runtime dependency inventory, private
  namespaces and an owned cgroup. The resource domain must verify the spawned
  bubblewrap process **and every launcher descendant** before Continue. The
  direct child PID is not necessarily the nested launcher's PID.
- On macOS, the signed/hardened launcher and JIT/runtime validation required by
  the native launch contract, plus the platform's total resource accounting.

The resource domain must terminate its entire owned tree and await its emptiness
after direct-child reap. A POSIX process group alone cannot reach bubblewrap's
nested session. The current test domain covers only the single-process fixture;
it is not a production cgroup or quota implementation. A bubblewrap block-fd
would release on EOF, so it is not an authorization substitute for Continue.

Before exec, `posix_spawn` replaces the environment, creates a new session,
resets signal state, maps only FD0..4 and closes ambient descriptors. Source FDs
are first relocated above FD4 to avoid aliasing during the ordered dup actions.
The Linux implementation uses glibc's `addclosefrom_np` (glibc >=2.34); selecting
and validating the supported Linux installer baseline remains required. macOS
uses SDK `POSIX_SPAWN_SETSID` and `POSIX_SPAWN_CLOEXEC_DEFAULT` (initial release
floor macOS 13.5). There is no shell, caller-defined environment or public raw
executable override.

A joinable supervisor thread polls startup, cancellation, child exit and logs.
Startup is bounded to at most 15 seconds. Logs retain at most 64 KiB per stream
(16 KiB default), with a combined one-second rate ceiling up to 1 MiB/s
(128 KiB/s default). Each poll also limits log-drain work. Raw log tails remain
untrusted bytes: rendering/redaction is a kernel presentation responsibility.
Lifetime App CPU and disk limits in the native record do not replace aggregate
resource policies. App workers may run indefinitely until shutdown or failure.

Cancellation and Drop kill the owned process group/domain, wait for the child,
and retain preparation/reservations until domain cleanup completes. The direct
child remains unreaped until group termination, preventing reuse of that PID
during normal cleanup. `spawn_blocking`, `wait_blocking`, `shutdown_blocking` and
Drop must run under the kernel's bounded blocking ownership. Cancellation clones
signal without waiting. The owner must never be detached onto an async runtime
whose shutdown could release its reservation before actual reap. OS wait may
remain blocked by an uninterruptible kernel operation; reservations deliberately
remain held in that case.

The test-only libc fixture links the production C record parser and continuation
function, then exercises real FD/environment/session behavior, SDK transport,
identity mismatch, withheld continuation, resource rejection, timeout, early
exit, log flooding, cancellation, Drop and panic cleanup. Its compilation uses
small temporary native files outside the repository and deletes them afterward.
It does not load Node or establish native sandbox, hostile App, signed runtime,
memory/quota overshoot, Linux namespace, or production activation acceptance.
