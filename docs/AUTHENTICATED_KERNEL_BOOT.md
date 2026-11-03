# Authenticated kernel boot (MP-08 / MP-09 / MP-10 / MP-11)

The reusable preinstall image belongs to the connected control plane. Its recipe,
compute provider, OS, tools, snapshot lifecycle and resource accounting live in
Cloud. OSS contains the release package and provider-neutral boot installer only.
The old release-coupled `prepare-worker-image.sh` is removed. Existing image
install/update tools remain for their installed-runtime consumers; they are not
an alternate preinstall-image builder.

An image contains `/usr/lib/chariox-boot/boot-release.mjs` and the independently
pinned raw Ed25519 public key (canonical base64) at
`/etc/chariox/boot/release-public-key`. Both are root-owned and read-only to the
runtime user. Neither comes from the downloaded release. Node is preinstalled.
The image contains no kernel release, enrollment input, runtime identity or user
account. Cloud keeps its opaque image reference separate from the selected
immutable release URL and `sha256:` manifest digest.

For every new allocation the control plane stages the one-time bootstrap input
privately, then runs as root:

```sh
node /usr/lib/chariox-boot/boot-release.mjs \
  https://release-host/immutable-release-id/ sha256:<manifest-digest>
```

The directory contains `release-manifest.json`, `release-manifest.sig`, and one
`artifacts/<name>` object for every manifest entry. The exact schema-2 manifest
bytes are digest-pinned and signed with Ed25519; every artifact has a signed
SHA-256 digest and an allowlisted installation path. The installer refuses HTTP,
credentials, query strings, fragments and redirects. Metadata and artifact bodies
are bounded, and artifact downloads are streamed. It never executes a downloaded
verifier, archive extractor, setup script or installer. It verifies all binaries,
attestation files and the service unit before publishing any executable facade.

The release installs under `/usr/lib/chariox/releases/<manifest-sha256>/`, with
`current` selected atomically after verified files and facades are present. The
only enabled entry is `chariox-managed-bootstrap.service`, whose signed source is
`deploy/managed-kernel/chariox-vm-bootstrap.service`. It starts the existing
`chariox-managed-bootstrap` supervisor without a worker/topology unit selector.
The VM remains the ordinary provider boundary. Reboots use the pinned installed
release and the existing bootstrap receipt; re-running the installer against an
installed allocation is refused. This entry is for fresh allocations, not an
upgrade or rollback mechanism. Download/verification failure removes staging and
never starts the unit; Cloud must settle failed-allocation deletion. A failed
activation remains an explicit operation failure, not an enrollment success.

S1 checks installation and refusal before enrollment. S2 owns provisioning the
real enrollment input, access readiness, preservation and restore. Without an
enrollment envelope or receipt the existing supervisor cannot launch a managed
kernel; starting its systemd unit alone does not establish enrollment acceptance.

## Release publication and trust

`scripts/package-boot-release.mjs` packages independently built kernel and
supervisor binaries, the signed common unit and builder attestation using the
existing schema-2 runtime manifest. It accepts a clean source checkout and
requires explicit binaries and signing key. Run it on the release signer, never
with a production private key on a lane builder. The production release service
runs the same packaging/signing step with its established Ed25519 signing setup,
publishes the immutable directory to HTTPS object storage, and hands Cloud its
URL and manifest digest. The preinstall image pins that service's public key by
a reviewed image recipe change. Routine releases change only the URL/digest.
Changing the trust root requires a new base image. No key supplied beside a
release can replace the image pin.

The lane drill is deliberately separate: an ephemeral TEST key signs two local
builds, and the candidate image pins only its TEST public key. The HTTPS test
server uses an ephemeral certificate with explicit CA trust on the validation
clones; verification is never disabled. The candidate is test-only and cannot be
promoted to production. The private TEST key is destroyed after the drill.

## MP-08 / MP-09 / MP-10 / MP-11 — protected local-disk enrollment

Cloud may supply managed bootstrap envelope schema 4 for a machine whose state
lives on its local disk. It uses the canonical protected bootstrap path and the
same signed installer, supervisor, grant exchange/confirmation, owner identity
and durable grant/receipt binding as managed enrollment. Schema 4 requires the
explicit repository root and rejects attached-volume serial/size fields. A
confirmed restart reuses its persisted identity and receipt. Schema 3 remains
for the existing attached-volume path. This private bootstrap envelope change
does not change the kernel client or relay wire shapes.
