# Managed remote-kernel image

The Hetzner adapter creates servers from one immutable snapshot ID. Build that
snapshot from the exact OSS revision before adding its numeric ID and release
digest to a Cloud provider profile. OpenShip deploys Cloud and the private
manager; it does not provision managed machines or build their images.

## Path-1 execution boundary

Path 1 allocates one disposable Cloud VM per isolated worker. That VM is the
provider security and filesystem boundary. After the signed kernel enrolls,
providers use the ordinary kernel launch path and normal developer-machine
filesystem behavior. The Path-1 service must not enable
`CHARIOX_MANAGED_PROVIDER_ISOLATION`, require `/usr/bin/bwrap`, or apply a
managed-only workspace allowlist merely because Chariox provisioned the VM.
The provider must not inherit equivalent restrictions from the managed
supervisor's systemd unit. Controls such as `ProtectSystem`, `ProtectHome`,
`PrivateTmp`, `NoNewPrivileges`, `RestrictSUIDSGID`, and `ReadWritePaths` must be
removed, topology-gated, or confined to a separate control-plane process when
they would change provider behavior from the ordinary worker user. A hardened
supervisor may not silently turn its child provider into a restricted
managed-only environment.

Bubblewrap may remain an inner defense for Docker slices or an explicitly
different legacy/shared-host topology. It is not part of a Path-1 provider run
and cannot be used as evidence of ordinary-kernel parity. Path-1 acceptance must
prove no Bubblewrap ancestor or managed-isolation marker, plus successful use
of arbitrary working directories and filesystem operations permitted to the
ordinary worker user. It also compares mount visibility, `/tmp`, writable
paths, process privilege flags, and permitted package/tool installation with an
ordinary-kernel control running the same reviewed build.

Root-owned signed releases, bootstrap/control state, broker state, relay and
Cloud credentials, resource limits, provider-resource deletion, and every
automatic-shutdown trigger remain mandatory. They must be protected through
service ownership, dedicated roots, bounded credentials, and process
environment hygiene without forking ordinary provider filesystem semantics.

## Release inputs

MP-07/MP-10/MP-11: use `deploy/managed-kernel/observe-image-preparation.sh`
with the same three arguments as `prepare-hetzner-image.sh` for a disposable
image-builder campaign. Both preparation and installation emit fixed public
phases before package provisioning, verification, pin admission, publication,
provider probes, rootless startup, registry pull/build, and freezing. Plain
progress uses shell builtins; preparation explicitly installs Node before the
existing Node verifier and canonical builder-pin comparator. Do not replace
canonical key identity with byte comparison or silently provision another pin.

For off-machine capture, provision Python and download the exact reviewed tools
before invoking the wrapper, then set
`CHARIOX_IMAGE_PREPARATION_OBSERVER_URL=https://<own-host>/path1-diagnostics/<campaign>`.
Start the matching receiver first. The wrapper starts a bounded transient
systemd shipper before installation, journals only allowlisted public events,
and flushes acknowledged receipts before stopping its own observer. It never
ships command arguments, package output, general journals, credentials, or key
bytes. Preparation errors retain their original exit status; missing final
acknowledgement prevents a successful observed campaign. Preserve the independent
outside watchdog and builder deadline. An abrupt builder loss can retain only
the last acknowledged phase; neither a phase nor an observer receipt establishes
release, session, or machine readiness. Collect the bootstrap download/package
steps separately before these tools are available.

Local nested-Docker preparation needs a dedicated host-filesystem bind mount
for `/var/lib/chariox-docker`; an overlay-backed container root may reject the
rootless overlay snapshotter. This is local drill setup, not a managed-image
storage-policy change. A local preparation pass does not diagnose an unobserved
paid timeout or close MP-07/MP-10 acceptance.

Use the OpenShip builder to build `chariox-kernel`, `chariox-managed-bootstrap`,
`chariox-relay`, `chariox-app-package`, and `chariox-app-storage` for `x86_64-unknown-linux-gnu` from an exact pushed
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
  --app-package <build-output>/chariox-app-package \
  --app-storage <build-output>/chariox-app-storage \
  --builder-attestation <build-output>/build-attestation.json \
  --builder-attestation-signature <build-output>/build-attestation.sig \
  --trusted-builder-public-key <openship-builder-public-key> \
  --signing-key <release-ed25519-private-key> \
  --source-repository "$(pwd)" \
  --source-commit "$(git rev-parse HEAD)" \
  --output <empty-release-directory>
```

Record the printed `sha256:` release digest. Keep both private keys private.
Copy only the generated root filesystem and separate copies of its release
public key and the pinned OpenShip builder public key to the image builder.
Keep the builder key outside the release root filesystem and supply it as
`CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY` for Path-1 preparation and upgrade.
The installer checks it independently against the packaged key and verifies
the builder attestation at every Path-1 release activation. It also installs
that public pin at `/etc/chariox/trusted-builder-public-key`,
outside the signed release tree, and the Path-1 bootstrap service passes this
path to the runtime for its own revalidation. A different existing pin is an
error during image preparation, not an implicit rotation. For an explicitly
authorized Path-1 upgrade across builder epochs, keep the current external pin
in `CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY` and supply the next external pin through
`CHARIOX_NEXT_TRUSTED_BUILDER_PUBLIC_KEY`. Omitting the latter preserves the
current pin. Both inputs must be independently trusted, root-owned regular
files outside the image. Release-signing key rotation still uses the optional
fifth argument to `upgrade-image.sh`.

The upgrade journals both public builder pins, switches the runtime pin while
services are stopped, and restores the previous pin before rollback startup.
Recovery checks the journal, installed pin and selected release before removing
transaction evidence. Legacy Path-1 transactions without a builder-pin journal
are refused as ambiguous; do not delete their journal or substitute a pin to
force recovery. Retain their public history and resolve the interrupted old
transaction with its matching reviewed tooling before attempting rotation.
The packager
verifies the detached builder signature, exact commit and tree IDs, target,
and all five staged binary digests
before it reads the release-signing key. The signed release retains the builder
attestation and records the full Git commit and tree IDs. Its systemd units and
slice context come from that exact Git object; working-tree changes and
untracked files cannot enter the release.

The signed release includes the privileged App storage helper and its service.
The installer derives its enrollment from the actual `chariox` OS UID/GID; it
never imports an App-supplied mount path, device, quota or command. The managed
kernel receives its own cgroup-v2 subtree with systemd `DelegateSubgroup`; the
rootless Docker subtree and broker remain separate. Storage uses the installed
helper and fixed ext4 images, so the image includes e2fsprogs. The helper shares
the host mount namespace and the kernel receives those mounts through its
existing service namespace. The dedicated Ubuntu24.04 storage drill covers
that seam; its first execution and the Ubuntu26.04 full image gate remain
required validation.

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
the trusted public key, and `deploy/managed-kernel/` to a task directory. The
image topology is a required caller-provided validation input. The production
managed image is the shared-host topology, so pass `shared_host` explicitly;
omitting it or using an unknown value fails closed instead of silently skipping
the Bubblewrap/provider-bind validation. A disposable Path-1 worker image must
likewise opt in explicitly with `path1`.

Run the production/shared-host preparation path with:

```sh
sudo env CHARIOX_MANAGED_PROVIDER_TOPOLOGY=shared_host \
  deploy/managed-kernel/prepare-hetzner-image.sh \
  <release-rootfs> \
  <release-digest> \
  <trusted-public-key>
```

For a disposable-VM Path-1 image, replace `shared_host` with `path1` and set
`CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY=<external-pinned-builder-public-key>` in
the `sudo env` command. Keep that file root-owned with mode `0600` for upgrades.
The key path must be outside the release rootfs;
omitting it fails before package installation or image mutation. Use the same
external pin for Path-1 `upgrade-image.sh` and parity collection's
`--kernel-builder-public-key` argument. The shared-host rollback path does not
require the new builder pin.

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

The broker runs as `chariox-docker` inside the rootless Docker daemon's user,
mount, and network namespaces. Its mount namespace contains RootlessKit's
private DNS resolver, which must use the matching network namespace. Otherwise
Docker CLI registry-auth requests fail even when a host-side `docker pull`
works. The namespace entry wrapper validates the daemon child PID, owner, and
namespace handles before invoking `nsenter`. The broker still uses its scoped
filesystem Unix socket; no new host TCP listener or Docker access is granted.
Image preparation verifies a minimal build through this same entrypoint,
with an explicitly digest-pinned external Dockerfile frontend and client-side
registry authentication. The probe forces `DOCKER_BUILDKIT=1`, so missing
BuildKit/buildx support fails acceptance instead of silently using the legacy
builder. A host-side image pull does not cover this failure.

The offline regression needs no Docker daemon or external network. Run it as
root in a disposable Linux test VM with Python 3, iproute2, and util-linux:

```sh
sudo python3 scripts/managed-rootless-network-drill.py \
  apps/kernel/slice-linux-docker/enter-rootless-docker-namespace.sh
```

It exercises the real helper against a private DNS fixture, checks command
exit propagation and parent-network isolation, and removes its processes and
temporary state. It complements, rather than replaces, fresh-image acceptance.

For each repository bind, the broker opens the published directory without
following symlinks, records its device and inode, and bind-mounts that open file
descriptor onto a broker-owned stable handle below
`/var/lib/chariox-docker/mount-handles`. Docker receives only that normal
mountpoint. A broker restart reuses a matching mount, while a rootless-daemon
restart recreates it in the new namespace only after the durable source record
still matches. Destroying the slice unmounts and removes its handles.

Provider processes on a disposable Path-1 managed VM are launched through the
ordinary kernel provider path. Normal Unix ownership and permissions determine
their filesystem access, including access to user-created directories and
repositories that were not part of initial context transfer. Kernel loopback,
Cloud, relay, bootstrap, and broker credentials must not be inherited by the
provider process.

Image acceptance launches a real provider through that ordinary path. It proves
broad developer-machine behavior, selected provider-account operation, absence
of control credentials, no `CHARIOX_MANAGED_PROVIDER_ISOLATION` marker, and no
Bubblewrap ancestor. A Bubblewrap isolation probe belongs only to a separately
selected slice or legacy/shared-host topology and must not gate Path-1 startup.

Broker-backed slice containers run in the dedicated rootless Docker daemon and
relax the outer seccomp, AppArmor, and system-path masks only so bubblewrap can
create its inner user, PID, and mount namespaces. Bubblewrap then drops every
capability, mounts a private `/proc` and `/dev`, disables further user
namespaces, and exposes only the approved provider profile and repository
roots. Ordinary local Docker slices retain Docker's defaults unless the user
enables the advanced compatibility option.

Ubuntu hosts with `kernel.apparmor_restrict_unprivileged_userns=1` must load
`apps/kernel/slice-linux-docker/chariox-slice-provider.apparmor` and set
`CHARIOX_SLICE_APPARMOR_PROFILE=chariox-slice-provider` for the worker kernel.
The policy keeps the outer container unconfined as compatibility mode already
requires, while explicitly permitting Bubblewrap to create its nested user
namespace. Compatibility-mode startup always runs the real provider isolation
probe and rejects the slice before use if the namespace cannot be created.

The first managed Docker slice builds its runtime image lazily from the complete
signed context, using the builder-attested kernel and relay binaries rather than
compiling them again on the managed host. Ordinary local source builds retain
the Cargo build path. The cache fingerprint covers the Dockerfile and all build
inputs; base images, Debian snapshots, the Cargo lock, and the provider npm
integrity lock are pinned. Host provider commands are installed with `npm ci`
from the same signed lock. Provider CLI policy lives in
`deploy/managed-kernel/provider-versions.env`: Codex 0.159.3, OpenCode 1.18.23
and Claude Code 2.1.212. Headed slice and publication builds verify their
installed CLIs against that policy. Update both toolchain manifests and npm
integrity locks together; `scripts/provider-cli-pins.test.mjs` rejects drift
in the manifests, resolved lock entries, build checks and publication labels.
This shared packaging policy supports MP-08/MP-11; source checks alone do not
close MP-10 live acceptance. The script enables the bootstrap and rootless Docker
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

## In-place release update (Path 1)

A Cloud-managed machine changes its kernel release in place, keeping `~/.chariox`
(plan, locked decisions for 2026-09-30). The machine runs
`deploy/managed-kernel/upgrade-image.sh` as root with the target signed rootfs:
it verifies the release, journals the transaction, migrates any legacy home,
switches `current` atomically, restarts the storage services and the kernel, and
rolls back automatically when the new kernel does not publish healthy presence
under `/home/chariox/.chariox/kernels/active`. A crash at any phase is recovered
by running the same command again.

Newly packaged releases use signed manifest schema 3 and declare
`managedUpdateEvidenceVersion: 1`. Cloud automatic updates require this target
capability. Their updater retains identity-bound, durable committed or rolled-back
results after recovery journal cleanup, so a restarted kernel can settle the exact
Cloud attempt. The newer verifier still accepts schema 2 for current-release
verification and explicit legacy rollback; Cloud automatic updates cannot select a
schema 2 target.

Legacy source runtimes are unsupported for an automatic transition to schema 3.
Their installed verifier rejects the new signed manifest before activation. Merely
replacing the updater script is insufficient: the legacy kernel does not durably
bind its attempt to the source release and Cloud identity or pass the update ID.
An operator transition that preserves `~/.chariox` and establishes an independently
verified evidence-capable runtime would be a separate prerequisite. That bridge
has not been implemented or validated. Reimaging is not the upgrade path. A fresh
schema 3 installation is only a starting point for disposable validation of the
automatic schema 3 A-to-B path.

The retained bootstrap envelope keeps naming the provisioned release. After
confirmation the kernel verifies its installed release against the receipt, and
the receipt's grant binding (schema 2) covers the machine's identity, not its
release, so an updated machine starts normally. Cloud authorizes the target
release and records it once the machine reports it; an update Cloud did not
authorize leaves Cloud's release record unchanged.

Deliberate managed rollback from local protocol 410 or later to a pre-Apps
protocol is blocked when durable Phase 1 state exists: installed App releases,
App storage, App tables or migrations. Initialized App tables count even when
empty: opening the 410 kernel database writes the Phase 1 schema, so an ordinary
410 host that has started its kernel requires the override for deliberate
downgrade even if no App was installed. Empty enrollment/storage roots alone do
not block a host whose database has not acquired App tables. Unreadable, corrupt or redirected state requires the override.
An operator may pass `--allow-apps-rollback` before the positional arguments to
`upgrade-image.sh`; this warns about unproven App state recovery and still
requires signed reciprocal transition policy. Preserve a state backup first.
Automatic recovery of a failed upgrade remains the existing transaction path
and does not pass through this deliberate rollback gate.

Release F Path-1 slices retain their original save/backup behavior until migrated.
The broker records `layoutKind: legacy-release-f` under its private
`legacy-layouts/` inventory, bound to the inspected home lineage, with the current container and image recorded. Signed worker refreshes and broker-recorded legacy saved images retain that lineage across recreation and restore.
Capture emits a warning in the home kernel diagnostics that the mixed home/image may contain credentials. Legacy saved-image proofs are separate from protected image proofs; they never admit protected capture.
Only pre-Apps release F image protocols (relay 58–68), without protected mounts
or environment markers, qualify. New slices and protected slices still require
the complete verified protected layout; losing its receipt never enables legacy
capture. Migrate intentionally to a new protected slice to separate credentials.

Pre-upgrade saved images have no broker image proof. For an already retained
legacy home only, restore preserves release F’s saved-image selection: the
kernel selects its saved-state image, and the broker requires a release F relay
label (58–68) with no protected marker. That compatibility fallback is not a
per-slice image proof. Newly captured legacy images have broker-owned proofs;
protected restores never use the release F fallback.

Managed upgrade enrolls App storage through the same root-owned enrollment path
as installation, publishes the signed App helper/package/unit links, and starts
the helper before the kernel. Rollback to a pre-Apps release disables the helper
and removes only these release links. Enrollment and durable App storage remain
intact; the rollback prints a notice that App storage is disabled until a later
Apps upgrade. Conflicting host links or enrollment fail rather than being adopted.
