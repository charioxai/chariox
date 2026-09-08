# Linux App storage

This is the production root helper and private kernel lease client. No ordinary
unit test starts it. Linux hosted acceptance is in `app-storage-linux.yml` and
uses actual fixed-capacity ext4 images plus the managed kernel service's mount
namespace/delegation settings. Source implementation and unsigned hosted test
execution do not establish enrolled runtime signing or embedded Node acceptance.

The initial platform is Ubuntu 24.04 x64, cgroup v2, kernel >=6.1, glibc >=2.34,
systemd >=254 and e2fsprogs 1.47. Broader distributions/arm64 need actual release
validation, including the managed image’s Ubuntu26.04 end-to-end gate. The fixed managed service uses `DelegateSubgroup=supervisor`; its
App subtree is separate from the rootless Docker unit and slice broker.

`/etc/chariox/app-storage.json` is installed by root, with schema
`chariox.app-storage-enrollment.v1` and an `owners` array of OS `uid`, `gid`, and
an exact delegated `cgroup_root`, and up to16 root-enrolled `kernel_database_paths`.
The managed database is `/var/lib/chariox/home/state/kernel.db`; an alternate
local kernel is enrolled by the installer, never by a socket request. The helper
uses the same `ReleaseStore::root_for_database` mapping as the kernel and rejects
a digest found in more than one enrolled store. The managed path is
`/sys/fs/cgroup/system.slice/chariox-managed-bootstrap.service/apps`.
The root-only `--prepare-managed-domain` mode runs as that unit's ExecStartPre,
verifies its own `.control` cgroup, and configures only the fixed App subtree.
It never accepts paths or service names over the socket.

`/usr/libexec/chariox-app-storage` uses the signed versioned release and the
installer’s atomic current selection. The helper pins its running root-owned
regular executable through `/proc/self/exe` for formatter re-exec at FD6; it
does not resolve an untrusted release path for that operation. The daemon and fixed formatter
mode are the only other commands. The private per-UID socket is
`/run/chariox-app-storage/u-UID.sock`, mode0600; its root-owned parent is0711.
SO_PEERCRED authenticates every connection and the client also requires root as
its server peer. A bounded4096-byte frame contains only IDs/generation plus a
strict `app-<32hex>` cgroup leaf, or the current connection's opaque lease ID. `attach_code` additionally carries
only the verified package digest and enrolled runtime inventory digest/revision.
There are no request paths, devices, commands, quota overrides or passed FDs.

The helper derives each installation directory beneath
`/var/lib/chariox-app-storage/u-UID/i-SHA256(owner-NUL-installation)`.
Ancestors and backing images are root-owned. Data has512MiB fixed capacity;
temporary storage has64MiB. The mounted roots are owned by the kernel UID with
mode0700, `noexec,nodev,nosuid`. Standard private file APIs operate directly on
ext4. Kernel resource-domain composition must retain this lease through broker
drain, close every mount directory pin, release storage while its exact cgroup
still exists and is empty, and only then remove the cgroup.

A root-directory lock serializes helper instances; inherited formatter FD5
retains that ownership while writing. FD3 is the preallocated image, FD4 is the
held root-owned mke2fs executable (closed at exec). FD6 pins the running helper
through its own re-exec and closes before mke2fs. The fixed formatter child
sets PDEATHSIG, a60s parent deadline,30s CPU lifetime, exact file-size limit, and
runs explicit4096-byte block units through execveat. `nodiscard` preserves the
posix_fallocate reservation. The helper admits at most64 installation roots,
32GiB promised capacity and8GiB additional host free space, counting every
promised byte not yet backed by actual allocation, including failed creations
and temporary images that need recreation. These are private initial policies.

Every image creation, formatting transition, deletion intent and mount lease is
journaled and fsynced. Loop association is atomic via LOOP_CONFIGURE with fixed
size/AUTOCLEAR; recovery scans by backing dev/inode rather than trusting a stale
loop number. Ext4 UUID/block capacity and actual mount ID/flags/device/owner are
checked before use or detach. Unmount is ordinary, never forced/lazy. Failure
retains the pending journal and reservation; failed preparations also remain in
the helper's owned recovery queue. cgroup-v2 immutable paths plus inode and boot
identity prevent cleanup from signalling a replacement cgroup. On disconnect,
the helper quiesces only the original bound domain and waits for emptiness.

The helper uses systemd Type=notify so kernel startup waits for completed
recovery and socket readiness. It must share the host mount namespace. Its systemd unit intentionally
avoids settings that introduce a private filesystem namespace. The kernel keeps
its existing namespace restrictions and adds the storage root to ReadWritePaths;
the hosted drill must demonstrate incoming mount propagation after that service
has already started. No installer/root-helper execution occurs on a developer's
machine during these tests.

Primary implementation references:
- [Linux descriptor mount API](https://man7.org/linux/man-pages/man2/mount_setattr.2.html)
- [Linux cgroup v2](https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html)
- [Linux 6.1 cgroup implementation: v2 disallows rename](https://raw.githubusercontent.com/torvalds/linux/v6.1/kernel/cgroup/cgroup.c)
- [systemd delegation and DelegateSubgroup](https://raw.githubusercontent.com/systemd/systemd/v255/man/systemd.resource-control.xml)
- [loop UAPI](https://man7.org/linux/man-pages/man4/loop.4.html)
- [mke2fs](https://man7.org/linux/man-pages/man8/mke2fs.8.html)
- [ext4 superblock](https://www.kernel.org/doc/html/latest/filesystems/ext4/super.html)

Code views are part of that same lease and journal. Before worker preparation,
the kernel retains a `VerifiedReleaseLease` over the exact archive and extracted
tree plus `EnrolledRuntime` over the signed graph. The helper reopens only its
enrolled sources, verifies archive digest and readonly source structure, and
creates nonrecursive bind views at fixed `package` and `runtime` names. Both are
readonly, nodev and nosuid; package is additionally noexec. The client compares
root device/inode to its already verified held descriptors. Runtime executables
and the exact eight target-specific platform libraries are opened from this
view and compared to enrolled file inodes; no host library directory is scanned.

`PreparedWorker::prepare_linux` accepts those two sealed leases and the retained
installation trust binding. It derives owner, installation, generation, entry
and declaration names; no caller can supply a launcher, bootstrap, mount path
or resource override. The kernel remains responsible for current trust and
approval fences before startup and before activation. Candidate update data
snapshot/promotion is a separate lifecycle obligation: this constructor alone
does not isolate writes made by an unactivated registration handler.

Code mount intent, original mountpoint inode, source inode and observed mount ID
are durable before a grant. The helper seals a detached nonrecursive `open_tree` clone with `mount_setattr`
before `move_mount` publishes it into the shared namespace, so propagated
views receive the required flags on their first attachment. Recovery handles
a crash after attachment but before mount-ID journaling. Only fully observed
sealed mounts are returned to a kernel. Native reap and broker drain
precede closing mounted file handles, helper release and cgroup removal. The
original enrolled runtime and release shared locks remain held through cleanup.

The helper FD ceiling is4096. A conservative64 live leases each retain at most
40 signed graph files plus12 runtime metadata/release/directory/cgroup handles;
64 client sockets,16 listeners and128 reserved global/transient descriptors
bring the conservative budget to3536; this is headroom, not a measured peak. The helper serializes operations, so tree walks and formatter
startup do not multiply per lease. No runtime cache or trust invalidation policy
is introduced. A regression fences both this unit limit and package archive,
file, depth and extracted-entry bounds against the Phase1 verifier defaults.
