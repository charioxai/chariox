# Chariox distribution plan (OSS)

Owner decisions of 2026-10-01. This document records how the open-source runtime
reaches users. Tooling: `apps/cli/scripts/compile.mjs` (#689),
`scripts/release-bundle.mjs` and `.github/workflows/release-bundle.yml` (#690),
`deploy/local-linux` (#599), `scripts/sign-macos-release.mjs` (#678) and the
codesigned runtime inventory (#681).

## Principles

- **One release bundle per platform**, built and signed by CI from one tagged
  commit. Phase 1 platforms: **macOS arm64** (`darwin-arm64`) and **Linux x86_64**
  (`linux-x64`).
- **Everything prebuilt.** Users need no Rust, Node, Bun or pnpm.
- **One version number** for the whole bundle and everything in it.
- **Keys stay with the owner.** CI signs with keys held in a protected GitHub
  environment. No agent or script creates, stores or prints a release key.

## Components

| Component | Source | In the bundle |
|---|---|---|
| Kernel | `apps/kernel` (`chariox-kernel`) | `bin/chariox-kernel` |
| Relay | `apps/relay` (`chariox-relay`) | `bin/chariox-relay` |
| CLI/TUI | `apps/cli`, compiled with Bun `--compile` | `bin/chariox` |
| App developer tool | `packages/app-package` (`chariox-app-package`) | `bin/chariox-app-package` |
| Runtime installer | `packages/app-runtime` (`chariox-app-runtime-install`) | `libexec/chariox-app-runtime-install` |
| App storage helper (Linux only) | `packages/app-runtime` (`chariox-app-storage`) | `libexec/chariox-app-storage` |
| Signed App runtime | native artifact, launcher, sandbox, SDK bundle | `runtime/` |
| Linux installer | `deploy/local-linux`, `deploy/release-bundle/install.sh` | `deploy/`, `install.sh` |

**The CLI/TUI executable.** `compile.mjs` embeds:
- the Bun runtime;
- every JavaScript dependency;
- OpenTUI's native renderer for the target;
- the tree-sitter worker and its grammars.

It reads no `.env` or `bunfig.toml` from the working directory. `chariox --version`
prints the bundle version. Each target is compiled on its own platform: CI uses a
Linux runner and a macOS arm64 runner. Bun was chosen over a Node single-executable
application because OpenTUI already requires Bun (`bun:ffi`).

**Not in the bundle.** These keep their own channels or a source checkout:
- the hosted control plane (`chariox-cloud`);
- slice images;
- provider CLIs, which users install from their vendors;
- the workflow publication server that the Rust `chariox-cli serve` launcher runs.

## Bundle layout

```
chariox-<version>-<platform>/
  manifest.json        schema chariox.release-bundle.v1
  manifest.sig         Ed25519 signature over manifest.json (128 hex)
  bin/                 chariox, chariox-kernel, chariox-relay, chariox-app-package
  libexec/             chariox-app-runtime-install, chariox-app-storage (Linux)
  runtime/             the signed App runtime release, unchanged
  deploy/              Linux only: local-linux/ installer and the units it installs
  install.sh           Linux only: checked root install step
  LICENSE
```

`manifest.json` records:
- `version`, `platform` and `sourceCommit`;
- `runtime.inventorySha256` and `runtime.publicKeyHex`;
- on macOS, the codesigning identity, team and notarization submission;
- every file's path, size, mode and SHA-256.

Two checks accept a bundle:
- `release-bundle.mjs verify` (Node; CI and contributors);
- `install.sh --check` (python3 and OpenSSL 3; users, before and during the root
  step).

Each check accepts a bundle only if all of these hold:
- `manifest.sig` verifies against the published release key;
- every file matches the manifest's size and SHA-256;
- no file was added;
- `runtime/runtime-inventory.json` is the inventory the manifest names, and its
  `runtime-inventory.sig` verifies with the runtime key the manifest names.

During root enrollment, `chariox-app-runtime-install` verifies the runtime again
against that key and digest.

## Channels

| Channel | What | Status |
|---|---|---|
| macOS `.pkg` | Signed (Developer ID Installer) and notarized package built from the `darwin-arm64` bundle | another agent builds it |
| Linux tarball | `chariox-<version>-linux-x64.tar.gz` plus `install.sh` (#599's root and user steps) | #690 |
| npm | `@chariox/app-sdk` and `@chariox/kernel-client` only | publish on release |
| crates.io | the Rust libraries (`chariox-event-protocol` and `chariox-aegs-sdk` already publish through `release-aegs-sdk.yml`) | per crate |
| `cargo install` | contributors only; not a user path | — |

**Linux install.**
1. Download and unpack the tarball.
2. As root: `sudo ./install.sh --user NAME --release-key <published key>`. It checks
   the bundle, runs `install-root.sh` with the runtime key and digest from the signed
   manifest, and installs `chariox` and `chariox-app-package` to `/usr/local/bin`.
3. As that user: `deploy/local-linux/install-user.sh install --kernel bin/chariox-kernel`.

## Signing and key custody

The keys and settings below live in the protected GitHub environment `release`:
- the owner is its required reviewer;
- deployments are limited to `v*` tags.

The owner creates every key, and only CI jobs in that environment can read it.

| Name | Kind | Signs or is used for |
|---|---|---|
| `CHARIOX_RELEASE_KEY` | secret, Ed25519 PKCS#8 PEM | bundle `manifest.json` |
| `CHARIOX_RUNTIME_RELEASE_KEY` | secret, Ed25519 PKCS#8 PEM | App runtime `runtime-inventory.json` (existing signer) |
| `MACOS_DEVELOPER_ID_P12`, `MACOS_DEVELOPER_ID_P12_PASSWORD` | secrets | Developer ID Application identity for #678 |
| `APPLE_NOTARY_KEY`, `APPLE_NOTARY_KEY_ID`, `APPLE_NOTARY_ISSUER` | secrets | App Store Connect API key for `notarytool` |
| `CHARIOX_RELEASE_PUBLIC_KEY` | variable, 64 hex | the published release key, checked against every bundle |
| `CHARIOX_RUNTIME_BUILDER_PUBLIC_KEY` | variable, SPKI PEM | the trusted runtime builder key |
| `CHARIOX_CODESIGN_IDENTITY` | variable | `Developer ID Application: <Name> (<TEAMID>)` |

**Two release keys.** The bundle key tells a user that this download is the
release. The runtime key is what the root installer and the kernels trust for App
runtime enrollment. Keeping them separate means a bundle key rotation does not
re-enroll runtimes.

**The published key.** The bundle public key is published in the README, on the
website and with every GitHub release. `install.sh` takes it only from the user,
never from the bundle.

**macOS order.** Codesigning changes bytes, so the steps run in this order:
1. #678 codesigns and notarizes the executables and a copy of the runtime input.
2. #681 signs the runtime inventory over the codesigned bytes.
3. The assembler checks #678's receipt against every executable.
4. CI signs the manifest.

**CI hygiene.**
- Key files are written with `umask 077` and removed on exit.
- The macOS keychain is temporary and deleted afterwards.
- Nothing prints a key.
- The workflow is dispatch-only and creates a draft release that the owner
  publishes.

**Rotation.** If the bundle key is compromised, the owner publishes a new key and
re-signs the current release. A runtime key rotation ships as a new runtime
generation signed by the new key. The signed manifest names the key that the root
step installs it with.

## Versioning

- One semantic version, `X.Y.Z[-pre]`, for the bundle. The tag `vX.Y.Z` must point
  at the commit CI builds, so the version and the source commit are fixed together.
- `manifest.json`, the bundle and tarball names, and `chariox --version` carry it.
- The runtime inventory records the same `sourceCommit`, and the assembler refuses
  a runtime built from another commit.
- The kernel's local protocol version is independent: clients compare protocol
  versions, not bundle versions. A protocol change still bumps the protocol version
  (AGENTS.md).
- The npm packages and crates are versioned per package. The bundle pins the
  versions it was built with through the lockfiles.

## Upgrade path

**Linux.** To upgrade, unpack the new bundle and run the same two steps.
- `install-root.sh` enrolls the new runtime as a new generation. The old generation
  stays while running workers hold its lease.
- The helper restarts only if its binary, unit or owners changed. A restart stops
  the Apps of running kernels, and the script says so.
- `install-user.sh` replaces the kernel and restarts the user unit. Apps recover
  through the kernel's startup recovery.
- Kernel state migrates forward only. Downgrades are not supported; restore from a
  backup instead.

**macOS.** A new `.pkg`, built by its own task, replaces the binaries and enrolls
the new runtime generation. Restarting the kernel relocks the vault, and the owner
unlocks it.

## Open items

- **Runtime builder in CI.** The release workflow consumes `runtime-input-<platform>`
  artifacts: the reviewed native artifact, launcher, sandbox and platform libraries,
  plus the builder's signed attestation. No CI job produces them yet. On Hetzner,
  `/w/linux-kernel/tools/build-runtime-release.sh` does it by hand.
- **Bun under the hardened runtime.** #678 gives the JIT entitlement only to
  `chariox-app-worker`. The Bun-compiled `chariox` needs
  `com.apple.security.cs.allow-jit` too.
- **Third-party notices** for the code embedded in the compiled CLI (Bun, OpenTUI,
  Solid and others).
- **Symbol stripping.** The release kernel is 129 MB with symbols. Stripped
  binaries and a separate symbol archive would shrink the download.

## Later channels

- Homebrew cask (from the `.pkg`).
- `darwin-x64` and `linux-arm64` bundles. OpenTUI and Bun already support them; the
  runtime and launcher need their native builds.
- Distribution packages (`.deb`, `.rpm`) wrapping the Linux bundle.
- An updater that checks for newer signed bundles.
