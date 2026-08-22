# Managed remote-kernel image

The Hetzner adapter creates servers from one immutable snapshot ID. Build that
snapshot from the exact OSS revision before adding its numeric ID and release
digest to a Cloud provider profile. OpenShip deploys Cloud and the private
manager; it does not provision managed machines or build their images.

## Release inputs

Build the `chariox-kernel` and `chariox-managed-bootstrap` release binaries for
`x86_64-unknown-linux-gnu` from the exact pushed OSS revision. Create an Ed25519
PKCS8 signing key outside the repository with mode `0600`, then package the two
binaries:

```sh
SOURCE_DATE_EPOCH="$(git show -s --format=%ct HEAD)" \
node scripts/package-managed-kernel-release.mjs \
  --kernel <linux-chariox-kernel> \
  --supervisor <linux-chariox-managed-bootstrap> \
  --signing-key <ed25519-private-key> \
  --output <empty-release-directory>
```

Record the printed `sha256:` release digest. Keep the signing key private. Copy
only the generated root filesystem and a separate copy of its public key to the
image builder.

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
architecture. It installs Node.js 22, Git and GitHub tooling, the exact pinned
Codex, OpenCode, and Claude Code releases, and the signed Chariox release. It
enables the bootstrap service without starting it, rejects runtime state, then
removes package caches, cloud-init identity, logs, SSH host keys, and the
temporary root authorization directory. It also removes the disposable-builder
marker before the snapshot is taken.

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
