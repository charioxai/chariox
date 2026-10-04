# Fresh Linux local Docker capture enrollment

`install-local-docker-dev.py` installs an explicit development enrollment for a
fresh local/rootful Docker Slice. The ordinary kernel keeps its existing user
identity and uses the normal Slice APIs through the framed broker channel.
This enrollment is an unsigned development configuration. It does not claim a
managed release signature or a managed control-plane deployment.

The installer requires root and an immutable, verified worker image with its
actual kernel SHA-256 and runtime-source-revision label. The worker runtime
revision is pinned separately from the installed helper/source manifest digest. The intended ordinary user must already have legitimate
access to `/run/docker.sock`, usually through the Docker group. The installer
does not grant socket access or alter account memberships. It verifies the
canonical root-owned socket and the same unmapped Linux engine at enrollment
and launch. Rootless engines, alternate sockets, remote contexts and user
namespace remapping are refused by this development path.

Supply reviewed SHA-256 values for the public host Docker CLI and a standalone
Node 22 executable, plus a verified public Buildx plugin. The Node input and every ancestor must be root-controlled;
its loader must work on the host. Only public, hash-pinned executable bytes and
tracked Slice source enter the immutable helper context. Docker configuration,
provider profiles, private keys and runtime state do not enter it.

```sh
sudo python3 deploy/local-linux/install-local-docker-dev.py \
  --allow-provider-sandbox-compatibility \
  --source /absolute/path/to/reviewed/chariox \
  --user ordinary-test-user \
  --worker-image sha256:VERIFIED_WORKER_IMAGE_ID \
  --worker-kernel-sha256 VERIFIED_WORKER_KERNEL_SHA256 \
  --worker-runtime-revision REVIEWED_EXACT_WORKER_RUNTIME_REVISION_LABEL \
  --docker-cli-sha256 REVIEWED_PUBLIC_DOCKER_CLI_SHA256 \
  --node-runtime /absolute/root-controlled/public/node \
  --node-runtime-sha256 REVIEWED_PUBLIC_NODE_SHA256 \
  --buildx-runtime /absolute/root-controlled/public/docker-buildx \
  --buildx-runtime-sha256 REVIEWED_PUBLIC_BUILDX_SHA256
```

The installer uses an isolated Docker configuration without authentication files
and a uniquely named owned Buildx builder, removed after the build attempt.
Buildx scratch uses a private directory beside the immutable source roots under
`/usr/lib/chariox/slice-local-dev`, because `/run` may be mounted `noexec`.
Normal completion and exceptions remove it. After a killed installer or power
loss, an administrator may remove its remaining `chariox-local-broker-build-*`
directory once that installer and its owned Buildx builder have stopped.
It does not install a global plugin or prune shared caches. The helper build verifies the CLI hash, loader and Python/archive dependencies.
A separate no-profile probe must negotiate successfully with the enrolled
engine before enrollment is published. The installer publishes one root-owned
per-user launcher, immutable public source, public image/engine pins and a
private control root. Existing incompatible enrollment or private directories
are refused rather than repaired. Updates need a reviewed enrollment procedure;
the installer is not a private-state migration tool.

Local DEV enrollment explicitly opts its workers into provider sandbox
compatibility, just as the topology fixture does. The installer requires
`--allow-provider-sandbox-compatibility` to acknowledge the grant: Docker
seccomp is disabled, system paths are unmasked, and the selected AppArmor
profile is used (default `unconfined`). The grant also adds `SYS_ADMIN`,
`NET_ADMIN` and `SYS_PTRACE` to the container capability bounding set. This
DEV engine is rootful and unmapped: container uid 0 maps to host uid 0, including
when the shipped setuid Bubblewrap helper runs. A DEV host must be selected with
this complete grant in mind. The broker applies this grant only to
its enrolled DEV workers; ordinary local slices still use
`slices.linux.allow_provider_sandbox_compatibility = true` as a separate opt-in.
The installer persists the acknowledgement as `providerSandboxCompatibility: true`
in the root-owned enrollment. Older enrollments without that field keep default
profiles; upgrading source alone never grants compatibility. Enabling it requires
an explicitly acknowledged, reviewed enrollment update as described above.
The isolation probe still runs and must succeed. On hosts restricting
unprivileged user namespaces, load `chariox-slice-provider.apparmor` and start
the home kernel with `CHARIOX_SLICE_APPARMOR_PROFILE=chariox-slice-provider`.

Use normal Create/Start/Save/Backup/Restore Slice operations after enrollment.
Fresh workers must pass the protected first-use identity retention barrier
before boot. Save and restore inventory only the exact verified home volume;
the broker never receives the whole Docker data directory. Restore admission
checks free space on the destination volume's actual filesystem.

### Worker image depth

Docker's overlay2 layer store refuses images deeper than 125 layers. Every Save
or Backup commits the container on top of the image it runs on, so a slice that
runs on a saved image (after Restore or a clone) gains one layer per capture. A
capture whose parent image already has 100 layers instead flattens the stopped
or paused container into a single layer with the same configuration
(`apps/kernel/slice-linux-docker/captured-image-depth.mjs`, applied by the
broker and by a kernel driving Docker directly). Saved images therefore never
exceed 100 layers. A flattened image shares no layers with its base, so a
flattening capture needs disk space for the whole root filesystem, and disk
admission reserves it. The cost repeats: a container keeps the image it was
created from, so every capture of a slice running on an image with 100 or more
layers flattens (and a desktop save keeps the slice paused for that copy) until
the slice is restored onto a flattened image.

Build the worker image with the repository's standard path: the provisioner's
Dockerfile build (`CHARIOX_SLICE_BUILD_IMAGE=always`, or `auto` when the image
is stale) from `apps/kernel/slice-linux-docker/docker/Dockerfile`. It produces
about 75 layers, so most save cycles stay ordinary one-layer commits. Validation
images assembled by adding one layer per changed file on top of a built image
can start near Docker's limit (a 124-layer candidate image failed Restore before
this bound existed), and every capture from such an image flattens. Squash such
an overlay into one layer, or rebuild from the Dockerfile, before enrolling it.

A fresh Slice without an explicit workspace uses its own named workspace
volume. Explicit host workspace binds and development mounts are refused;
they are not replaced with a blank workspace. Home capture does not imply
workspace-volume cloning. Legacy Slices and macOS capture remain unsupported
by this Linux path and fail closed, preserving existing saved state. Acceptance
requires actual ordinary-user capture/restore and failure/restart proofs on the
selected source and artifacts; enrollment alone is not that proof.

# Local Docker admission locks

Every kernel sharing a Docker engine uses the same two host locks:
`/tmp/chariox-docker-memory-admission.lock` and
`/tmp/chariox-docker-disk-admission.lock`. They must be empty root-owned regular
files with one link and no group/other writes. The provisioner uses mode 0444,
so ordinary kernels can open and lock them but cannot replace them in sticky
`/tmp`. It preserves an existing safe inode, including an active lock.

## macOS setup

The Chariox macOS pkg installs the root LaunchDaemon
`/Library/LaunchDaemons/dev.chariox.docker-admission-locks.plist` and runs the
shared provisioner before starting the kernel. At boot, launchd runs it again
because `/tmp` is volatile. macOS resolves `/tmp` to `/private/tmp`.
The daemon uses `/usr/bin/python3`; install the Apple Command Line Tools if that
interpreter is unavailable.

For a developer checkout, do this once as an administrator before starting the
Room or enrolling the developer App runtime:

```sh
sudo /usr/bin/python3 deploy/local-macos/install-docker-admission-locks.py
```

The developer runtime release receipt prints this command as
`provisionDockerAdmissionLocks` and prefixes its `enroll` command with it, so
runtime enrollment also installs boot provisioning. macOS App storage
uses the runtime's APFS implementation, so there is no Linux storage helper to
install here. The command installs the provisioner under
`/usr/local/libexec/chariox/`, provisions immediately, and enables and loads the
root boot daemon. No per-App privilege step is needed.

If a pkg is already installed, provision missing locks immediately with:

```sh
sudo /usr/bin/python3 /usr/local/libexec/chariox/provision-docker-admission-locks.py
```

Boot errors go to `/var/log/chariox-docker-admission-locks.log`.
`sudo launchctl print system/dev.chariox.docker-admission-locks` shows the daemon
and its last exit status. After an administrator repairs an unsafe lock, rerun
the setup command or reinstall the pkg to retry and restore boot provisioning.

## Unsafe locks from older kernels

The script refuses a user-owned lock, link, writable lock, or nonempty lock. It
never adopts or deletes it. First stop **all** kernels using this Docker engine,
including root, developer and other users' kernels. An administrator must then
inspect and remove only the unsafe admission lock files and rerun setup. Never
unlink a lock while any kernel can hold it: replacing its inode permits two
kernels to admit resources independently. Rebooting after installing the daemon
also clears legacy locks in volatile `/tmp`.

Uninstallation unloads the daemon and removes its program and plist. It keeps
the admission locks, since a developer kernel or another installation may still
hold their inodes; `/tmp` clears them on reboot.

## Linux

`deploy/local-linux/install-root.sh` and the managed-kernel installer deliver
`chariox-docker-admission-locks.service` and the same
`deploy/local-linux/provision-docker-admission-locks.py`. The service provisions
at boot before managed kernel activation. Manual repair follows the same rule
about stopping every kernel before removing an unsafe inode.
