# Fresh Linux local Docker capture enrollment

`install-local-docker-dev.py` installs an explicit development enrollment for a
fresh local/rootful Docker Slice. The ordinary kernel keeps its existing user
identity and uses the normal Slice APIs through the framed broker channel.
This enrollment is an unsigned development configuration. It does not claim a
managed release signature or a managed control-plane deployment.

The installer requires root and an immutable, verified worker image with its
actual kernel SHA-256. The intended ordinary user must already have legitimate
access to `/run/docker.sock`, usually through the Docker group. The installer
does not grant socket access or alter account memberships. It verifies the
canonical root-owned socket and the same unmapped Linux engine at enrollment
and launch. Rootless engines, alternate sockets, remote contexts and user
namespace remapping are refused by this development path.

Supply reviewed SHA-256 values for the public host Docker CLI and a standalone
Node 22 executable. The Node input and every ancestor must be root-controlled;
its loader must work on the host. Only public, hash-pinned executable bytes and
tracked Slice source enter the immutable helper context. Docker configuration,
provider profiles, private keys and runtime state do not enter it.

```sh
sudo python3 deploy/local-linux/install-local-docker-dev.py \
  --source /absolute/path/to/reviewed/chariox \
  --user ordinary-test-user \
  --worker-image sha256:VERIFIED_WORKER_IMAGE_ID \
  --worker-kernel-sha256 VERIFIED_WORKER_KERNEL_SHA256 \
  --docker-cli-sha256 REVIEWED_PUBLIC_DOCKER_CLI_SHA256 \
  --node-runtime /absolute/root-controlled/public/node \
  --node-runtime-sha256 REVIEWED_PUBLIC_NODE_SHA256
```

The helper build verifies the CLI hash, loader and Python/archive dependencies.
A separate no-profile probe must negotiate successfully with the enrolled
engine before enrollment is published. The installer publishes one root-owned
per-user launcher, immutable public source, public image/engine pins and a
private control root. Existing incompatible enrollment or private directories
are refused rather than repaired. Updates need a reviewed enrollment procedure;
the installer is not a private-state migration tool.

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
