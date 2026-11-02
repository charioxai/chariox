# Chariox distribution

MP-08 / MP-09 / MP-10 / MP-11. A release bundles the kernel, relay, compiled
CLI, package tool and the WASM App worker built from one source commit. The
kernel opens `chariox-app-wasm-worker` beside its own executable. App execution
needs no separately enrolled engine, dynamic engine library or root storage
helper. The former runtime inventory and builder-input jobs are retired.

## Bundle contents and trust

`bin/` contains `chariox`, `chariox-kernel`, `chariox-relay`,
`chariox-app-package` and `chariox-app-wasm-worker`; Setup may also be present.
`share/chariox/slice-build-context/` contains committed provisioner sources and
every Dockerfile input. Linux bundles include the local installer and browser
setup; macOS distributes a signed installer package. Licenses travel with the
corresponding artifacts.

The release manifest records semantic version, platform, source commit, and
every file's path, size, mode and SHA-256. An Ed25519 release signature covers
the exact manifest. Verification rejects altered, missing, linked or additional
files. A macOS bundle also requires a matching Developer ID/notarization receipt
for every executable. Platform signing happens before the release manifest is
signed, because it changes binary bytes.

`scripts/release-bundle.mjs` exports the committed slice context, assembles the
bundle, signs it and verifies it. `deploy/release-bundle/install.sh --check`
verifies Linux bundles with Python and OpenSSL before host changes. The release
public key comes from Chariox's release authority, never from bundle contents.
Private signing material belongs only in the protected release environment.

Managed Linux images use the builder attestation and signed image manifest in
`scripts/build-managed-kernel-release.mjs`,
`scripts/package-managed-kernel-release.mjs` and
`deploy/managed-kernel/verify-image-release.mjs`. Their App artifact set is the
package tool and the adjacent WASM worker. Kernel placement and provider logic
remain outside those binaries' App execution contract.

## Installation

Linux's privileged step installs verified Chromium, its namespace policy,
required native dependencies and host admission locks, then enables lingering
for the ordinary kernel user. It does not enroll an App engine or private
storage owner. `install-user.sh` installs the kernel and adjacent WASM worker in
the user's bin directory and starts the delegated systemd user unit. App data
lives under the kernel's own state and is owned by its lifecycle service.

The macOS package validates matching Mach-O architectures, carries the kernel,
worker and slice context, provisions host admission locks and restarts the
console user's kernel LaunchAgent. The native worker has the hardened runtime
and the JIT entitlement its containment entry requires. The compiled CLI and
Setup have their separately reviewed entitlements.

One semantic version identifies the bundle and source tag. Local/peer protocol
versions are independent and follow the shared protocol bump rules. Updating
replaces the kernel and adjacent worker together; the kernel owns App recovery.
Unsupported older installations are re-provisioned rather than kept through
runtime compatibility fallbacks.

## Release validation

Run package/signature/tamper checks and the real ordinary/managed installation
paths. Verify install/open, Excalidraw and Univer edit/save/reopen, update
recovery, capability and native-containment refusals, and cleanup through built
clients and the real kernel/relay. Source fixtures do not establish live
acceptance. Record exact heads, binary hashes, screenshots and logs outside the
repository. Run CI only when authorized for release.
