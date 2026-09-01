# Managed remote-kernel image

The Hetzner adapter creates servers from one immutable snapshot ID. Build that
snapshot from the exact OSS revision before adding its numeric ID and release
digest to a Cloud provider profile. OpenShip deploys Cloud and the private
manager; it does not provision managed machines or build their images.

## Release inputs

Use the OpenShip builder to build `chariox-kernel`, `chariox-managed-bootstrap`,
and `chariox-relay` for `x86_64-unknown-linux-gnu` from an exact pushed
OSS revision. The builder must hold a dedicated Ed25519 PKCS8 attestation key
outside the repository with mode `0600`. Its build command archives the Git
object into a new temporary directory, runs a locked one-job release build, and
emits the binaries with a detached source and artifact attestation:

```sh
node scripts/build-managed-kernel-release.mjs \
  --source-repository "$(pwd)" \
  --source-commit "$(git rev-parse HEAD)" \
  --builder-signing-key <openship-builder-ed25519-private-key> \
  --output <new-build-output-directory>
```

Create a separate Ed25519 PKCS8 release-signing key outside the repository with
mode `0600`. Package only builder outputs whose attestation verifies with the
pinned OpenShip builder public key:

```sh
SOURCE_DATE_EPOCH="$(git show -s --format=%ct HEAD)" \
node scripts/package-managed-kernel-release.mjs \
  --kernel <build-output>/chariox-kernel \
  --supervisor <build-output>/chariox-managed-bootstrap \
  --relay <build-output>/chariox-relay \
  --builder-attestation <build-output>/build-attestation.json \
  --builder-attestation-signature <build-output>/build-attestation.sig \
  --trusted-builder-public-key <openship-builder-public-key> \
  --signing-key <release-ed25519-private-key> \
  --source-repository "$(pwd)" \
  --source-commit "$(git rev-parse HEAD)" \
  --output <empty-release-directory>
```

Record the printed `sha256:` release digest. Keep both private keys private.
Copy only the generated root filesystem and a separate copy of its release
public key to the image builder. The packager verifies the detached builder
signature, exact commit and tree IDs, target, and all three staged binary digests
before it reads the release-signing key. The signed release retains the builder
attestation and records the full Git commit and tree IDs. Its systemd units and
slice context come from that exact Git object; working-tree changes and
untracked files cannot enter the release.

## Disposable Hetzner builder

Create one x86 Hetzner server from the `ubuntu-26.04` system image. Give the
server, its temporary SSH key, and its temporary SSH firewall the label
`chariox.dev/managed-image-builder=true`. Limit inbound TCP 22 to the operator's
current address. Do not attach the final outbound-only managed-machine firewall
to this builder.

Before copying release files, write this exact marker on the disposable server:

```text
managed-remote-kernels-image-builder-v1
```

Store it at `/.chariox-managed-image-builder`. Copy the release root filesystem,
the trusted public key, and `deploy/managed-kernel/` to a task directory. Run:

```sh
sudo deploy/managed-kernel/prepare-hetzner-image.sh \
  <release-rootfs> \
  <release-digest> \
  <trusted-public-key>
```

The preparation script refuses an unmarked host or the wrong OS and
architecture. It installs Node.js 22, Docker, Git and GitHub tooling, the exact
pinned Codex, OpenCode, and Claude Code releases, and the signed Chariox
release. It disables and masks the rootful Docker units, verifies both are
inactive, rejects any live owner or unexpected file at the rootful socket path,
and removes only the stale socket inode Ubuntu 26.04 can leave behind after
shutdown. It then validates a rootless daemon owned by the separate
`chariox-docker` principal. Neither the kernel nor its
provider and shell descendants can open that daemon socket. At boot the
bootstrap supervisor claims a single broker connection before any provider can
start. The broker socket is owned by `chariox-docker:chariox-slice`, has mode
`0660`, and lives below the setgid, non-listable
`/var/lib/chariox-slice-share/.broker-private/control` directory. The broker
immediately removes its listener after the bootstrap claim and accepts only
bounded slice operations and host paths under `/var/lib/chariox-slice-share`.
The credential vault remains under the private `chariox` home. Docker and the
broker are soft dependencies: kernel bootstrap still runs if either is
unavailable, while slice operations report Docker as unavailable.

The broker runs as `chariox-docker` inside the rootless Docker daemon's user
and mount namespaces, but not its network namespace. The namespace entry
wrapper validates the daemon child PID and owner before invoking `nsenter`.
For each repository bind, the broker opens the published directory without
following symlinks, records its device and inode, and bind-mounts that open file
descriptor onto a broker-owned stable handle below
`/var/lib/chariox-docker/mount-handles`. Docker receives only that normal
mountpoint. A broker restart reuses a matching mount, while a rootless-daemon
restart recreates it in the new namespace only after the durable source record
still matches. Destroying the slice unmounts and removes its handles.

Every provider process on a managed host is launched through the root-owned
`/usr/bin/bwrap` boundary. Its namespace masks the kernel home, broker and
Docker control paths, runtime logs, and unrelated host filesystems; it binds
only the selected provider profile, explicit runtime files, and canonical
repository roots. Managed startup fails closed when the wrapper or an approved
mount is missing. Kernel loopback transport uses a one-time local token outside
provider-visible mounts, and provider children receive neither that token nor
Cloud, relay, bootstrap, or broker credentials.

For image acceptance, set `CHARIOX_MANAGED_PROVIDER_ISOLATION_PROBE=1` for one
bootstrap. Startup then authenticates to the real local kernel with its one-time
token, creates a real session, and launches Codex through the production
bubblewrap wrapper. The wrapper proves write access to only the selected account
and repository, denies kernel state, slice-share and broker paths, host `/proc`,
and an unselected repository, and verifies that cross-mount hardlinks fail. The
kernel is stopped if any assertion fails. The probe token is removed from the
environment after the one-shot check and is never written as persistent state.

Broker-backed slice containers run in the dedicated rootless Docker daemon and
relax the outer seccomp, AppArmor, and system-path masks only so bubblewrap can
create its inner user, PID, and mount namespaces. Bubblewrap then drops every
capability, mounts a private `/proc` and `/dev`, disables further user
namespaces, and exposes only the approved provider profile and repository
roots. Ordinary local Docker slices retain Docker's defaults unless the user
enables the advanced compatibility option.

The first managed Docker slice builds its runtime image lazily from the complete
signed context, using the builder-attested kernel and relay binaries rather than
compiling them again on the managed host. Ordinary local source builds retain
the Cargo build path. The cache fingerprint covers the Dockerfile and all build
inputs; base images, Debian snapshots, the Cargo lock, and the provider npm
integrity lock are pinned. Host provider commands are installed with `npm ci`
from the same signed lock. The script enables the bootstrap and rootless Docker
services; the broker stays disabled and bootstrap republishes its one-claim
endpoint from a privileged prestart on each supervisor restart. The script
rejects runtime state, then removes package caches,
daemon test state, cloud-init identity, logs, SSH host keys, and the temporary
root authorization directory. It also removes the disposable-builder marker
before the snapshot is taken.

## Snapshot and cleanup

After the preparation command succeeds, use the Hetzner API to shut down the
builder and wait for the action to finish. Create an independent snapshot with
these labels:

- `chariox.dev/managed-image=true`
- `chariox.dev/runtime-release-a=<first 32 lowercase hex characters>`
- `chariox.dev/runtime-release-b=<last 32 lowercase hex characters>`
- `chariox.dev/source-revision=<40 lowercase hex characters>`

Hetzner label values are limited to 63 characters, so the 64-character release
digest is split across two ordered labels. Concatenating `runtime-release-a` and
`runtime-release-b` must reproduce the hexadecimal portion of the signed
`sha256:` release digest exactly.

Wait for the image action to finish, record the numeric image ID and its x86
architecture, then delete the builder, temporary firewall, and temporary SSH
key. If image creation fails, delete those temporary resources and do not add an
image ID to Cloud configuration.

The final managed-machine firewall must have no inbound rules. It must allow the
kernel's outbound HTTPS and WSS traffic. Put that firewall's numeric ID, the
snapshot's numeric ID, and the signed release digest into the immutable Hetzner
profile shared by Cloud and the infrastructure manager.
