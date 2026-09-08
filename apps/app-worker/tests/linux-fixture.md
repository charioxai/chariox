# Hosted Linux native containment fixture

`.github/workflows/app-worker-native-test.yml` is separate from the long Node
runtime build. It compiles the production C launcher and a test-only libc
runtime, plus bundled bubblewrap, on a disposable Ubuntu 24.04 x64 runner.
No Node source, libnode build, Docker image or local Docker execution is needed.
Node only coordinates the test from outside the App boundary.

`sandbox.lock.json` pins the official bubblewrap 0.12.0 release archive, byte
size and SHA-256. This release fixes an absolute-symlink escape during sandbox
setup; an arbitrary host-installed older binary is not an equivalent fixture.
The small source archive is verified before extraction and compiled with one
job. gcc, libc headers, binutils, Meson, Ninja, libcap headers, AppArmor and
existing util-linux/systemd supply the remaining hosted dependencies. This is
a recorded development fixture, not a reproducible signed release bundle.

The hosted wrapper provisions one named systemd service with 1 GiB memory,
zero swap, one CPU, 64 tasks and a 120-second lifetime. That cgroup exists before
the fixture's root mount setup, coordinator or any worker starts. Setup runs in
a separate mount namespace with private propagation. Small, owned tmpfs mounts
provide package/data/tmp with `noexec,nodev,nosuid`; package is read-only.
Runtime is read-only and executable. These fixture byte ceilings do not validate
the eventual installer's persistent storage quotas or production resource policy.

An AppArmor exception permits user namespaces only for the exact owned bundled
bubblewrap executable on that disposable VM. No global AppArmor switch or userns
sysctl is disabled. The profile is removed after the fixture. The coordinator
clears supplementary groups and drops to the runner UID/GID before executing
bubblewrap. No root-directory handle enters the worker. FD31 carries only a
deliberate fixture sentinel, so the production native descriptor filter is
tested independently of the coordinator's filtering.

Bubblewrap unshares user, mount, PID, network, IPC, UTS and cgroup namespaces;
disables further user namespaces; drops capabilities; starts a new session;
and kills the sandbox when its parent dies. `--as-pid-1` makes its reported
child PID the exact worker, which cannot create subprocesses under the native
policy. The empty root contains only:

- `/app/package`, `/app/data`, `/app/tmp`, `/runtime`;
- exact libc and ELF-interpreter file binds;
- `/dev/null` as the sole device;
- the harmless `/usr/bin/false` executable, to prove an actual existing
  executable is denied rather than merely absent.

The root is remounted read-only. Host `/proc`, home, `/run`, credentials and
general system-library trees are absent. SDK FD3 and startup FD4 use the exact
production ABI; fixture-only FD5 receives bubblewrap's bounded JSON status.
The native loader closes extra descriptors and installs its production seccomp
policy before it reports readiness. The coordinator checks the worker's actual
namespace identities, UID/GID/groups, zero capabilities, `no_new_privs`, seccomp,
cgroup membership and mount flags from the host before sending continuation.
It also confirms that no runtime constructor has executed yet.

The libc probe exercises private I/O, descriptor copying, actual watch-event
delivery, pthread create/join, bidirectional FD3 traffic, and denied host paths,
escaped watches, executable private mappings, raw network, fork, exec and
foreign signals/resource changes. A data mount deliberately missing `noexec`
must be rejected before readiness. A separately linked test-only weakened
policy must be detected by the same hostile native probe. Installed Apps cannot
choose that binary, policy, bootstrap or fixture mode.

On success the workflow uploads `native-linux.json` with the exact commit,
binary hashes, bubblewrap provenance, observed OS/kernel/compiler, namespace
and mount facts, resource settings and check results. It publishes no executable
artifact. The owned service, profile, mounts and scratch are cleaned up.

This is Linux x64 native boundary and explicit provisioning evidence only.
Installer integration, Linux arm64, signed artifacts, the real embedded Node
and SDK bootstrap, and production resource/quota validation remain required.
In particular, libc FD3 read/write does not establish all libuv/SDK transports.

Primary references: [bubblewrap 0.12.0 release](https://github.com/containers/bubblewrap/releases/tag/v0.12.0),
[pinned bubblewrap options](https://github.com/containers/bubblewrap/blob/v0.12.0/bwrap.xml),
[pinned setup and descriptor ordering](https://github.com/containers/bubblewrap/blob/v0.12.0/bubblewrap.c).
