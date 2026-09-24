# Linux resource domain

These private production modules implement cgroup-v2 admission, fixed bubblewrap
arguments, host observations before native Continue, and retained process-tree
cleanup. There is no public constructor accepting executable paths or a
`verified` flag. An enrolled runtime and private storage lease must still supply
the immutable executable/library graph and mounts before a production factory
can connect this machinery to installation activation.

The initial baseline is Linux kernel 6.1 or newer, glibc with
`posix_spawn_file_actions_addclosefrom_np` (2.34 or newer), cgroup v2 with delegated
cpu/memory/pids controllers, and the pinned bundled bubblewrap 0.12.0. The first
hosted acceptance target is Ubuntu24.04 x64. Source support does not claim that
other distributions or arm64 have passed the platform gate. User-namespace
admission on systems enforcing AppArmor needs the installed executable's exact
profile; a global user-namespace/AppArmor disable is not a supported setup.

The delegated empty inner cgroup is installer-owned input. Its manager runs in a
sibling leaf within the same delegation; the kernel owner needs write access to
`cgroup.procs` at their common ancestor as well as at the destination, as required
by [cgroup v2 migration rules](https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html#delegation-containment).
Outer resource-limit controls remain installer-owned. Each App receives a random
private child with memory.max 512MiB, memory.swap.max 0, memory.oom.group 1,
pids.max 64, and cpu.max 100000/100000. The small
native entry inherits only the normal worker FD0..4 plus FD5=cgroup.procs and
FD6=the held bubblewrap executable. It joins the leaf before exec or fork and
closes both setup descriptors before executing bubblewrap. No App code is linked
into this entry. The supervisor does not migrate an already-forking workload.

After the native worker returns the exact launch identity on FD4, host observation
requires its expected executable inode, private PID1, exact parent and cgroup,
all seven separate namespaces, zero effective/permitted/inheritable/ambient
capabilities, no supplementary groups, no-new-privileges and seccomp. The four
App roots are fixed; package and runtime are read-only, package/data/tmp are
noexec, and all four are nodev/nosuid. Only individual approved runtime system
libraries and /dev/null enter the namespace. The native launcher independently
checks roots and installs its syscall policy before it replies. The resource
owner rechecks cgroup limits/events while running; any stop kills the entire
owned cgroup and waits for populated0 before releasing mount/runtime leases.

The dedicated `app-domain-linux.yml` fixture builds the small native sources,
reuses the pinned bubblewrap build, and compiles only the Rust App runtime tests.
It creates disposable noexec tmpfs mounts and a delegated subtree inside a
bounded root service, then runs the actual Rust supervisor as the ordinary kernel
owner with cleared supplementary groups. Its tests exercise immediate fork
inheritance before observation, descriptor closure, actual native constructor
ordering, namespace inspection, private copy/watch I/O, raw-network/process/host
denials, and empty-cgroup cleanup. The outer execution limit is1GiB/no-swap/1CPU/
128tasks/120s; each App still receives the production512MiB/64task limit.

This is an unsigned provisioning fixture, not proof of embedded Node behavior,
signed artifact enrollment, persistent Linux storage, recovery of orphaned
cgroups after a kernel crash, or installation-wide resource admission. Persistent
storage must come from a bounded filesystem owned by the installer/kernel
resource service; the fixture's disposable tmpfs is not a persistent-storage
implementation. Host root/image provisioning remains narrow installer work,
separate from the Docker slice broker and from user-facing App configuration.
