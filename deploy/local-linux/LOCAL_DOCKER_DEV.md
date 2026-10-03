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
profile is used (default `unconfined`). The broker applies this grant only to
its enrolled DEV workers; ordinary local slices still use
`slices.linux.allow_provider_sandbox_compatibility = true` as a separate opt-in.
The isolation probe still runs and must succeed. On hosts restricting
unprivileged user namespaces, load `chariox-slice-provider.apparmor` and start
the home kernel with `CHARIOX_SLICE_APPARMOR_PROFILE=chariox-slice-provider`.

Use normal Create/Start/Save/Backup/Restore Slice operations after enrollment.
Fresh workers must pass the protected first-use identity retention barrier
before boot. Save and restore inventory only the exact verified home volume;
the broker never receives the whole Docker data directory. Restore admission
checks free space on the destination volume's actual filesystem.

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
