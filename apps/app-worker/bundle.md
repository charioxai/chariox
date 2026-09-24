# Unsigned runtime bundle assembly

`scripts/package-app-runtime.mjs` combines an existing native build with the
committed trusted bootstrap and complete pinned SDK source graph. It never
downloads, compiles, signs, enrolls, installs, or executes native/App code. The
native compiler and its successful-build receipt remain unchanged: changing
bootstrap or SDK files changes the **bundle digest**, without invalidating a
matching native binary build.

From a clean committed checkout with the native artifact's source commit in
local Git history:

```sh
node scripts/package-app-runtime.mjs assemble \
  --native-dir /absolute/builder-scratch/native-artifact \
  --output /absolute/builder-scratch/new-bundle \
  --target linux-x64
node scripts/package-app-runtime.mjs verify \
  --directory /absolute/builder-scratch/new-bundle
```

The target is one of the four entries in `runtime.lock.json`. The native
directory contains exactly its original `artifact-manifest.json`, Node library,
embedder library, and `NODE-LICENSE`. Both source and native inputs must remain
unchanged and exclusively controlled by the trusted builder throughout assembly.
Their directory ancestry and files must be owned by the builder or root, with
no writes granted to other users. Root-owned sticky temporary ancestors are
permitted; the output's immediate parent must be owned by the builder and not
writable by other users. Output is a new directory outside all repositories
and native inputs. Links, special files, unlisted files, empty subdirectories,
and replacement of an existing output are rejected.

These are path-based build operations, with opened regular files retained while
hashing/copying. They do **not** defend against hostile concurrent processes
running as the builder, ACLs that violate exclusive ownership, or replacement of
ancestor directories by a privileged process. The kernel installer must use its
separate verified, descriptor-anchored storage and immutable artifact contract.

The bundle includes:

- Both native libraries and Node's license, preserving their original digests.
- The original native manifest byte for byte as `native-artifact-manifest.json`.
- `bootstrap.cjs` and `bootstrap-config.cjs` at the runtime root.
- `sdk/package.json` and every runtime/declaration file listed in
  `bundle.lock.json`, under `sdk/src/`, plus Chariox's license.
- Both runtime/bundle locks and `bundle-manifest.json`.

The SDK has no external dependencies. Its actual source directory must exactly
match the lock inventory and its package exports/version must match the pinned
SDK contract. Bootstrap resolves this fixed runtime graph; App paths cannot
provide an alternative SDK. The bundle does not include the native launcher or
Linux namespace provisioner. Release packaging must combine these with the
complete runtime under the supervisor's verified launch contract.

Inventory is streamed and capped at 32 files, 96 total directory entries,
256 KiB per trusted source/manifest and 512 MiB for the complete bundle. Files
have sorted relative paths, sizes, SHA-256 hashes, and intended publication mode
`0444`. This mode is an instruction for publication/extraction; `verify` checks
bytes and safe input permissions, **not immutable storage**. Artifact transport
may change actual modes. The builder still owns the files.

`bundleDigest` is SHA-256 of the UTF-8 JSON manifest body with recursively sorted
object keys and no whitespace, excluding the `bundleDigest` field itself. This
internal build identity is not the `.cxapp` RFC8785 signing format. The manifest
records exact source hashes for bundled files and assembly/native tooling;
identical committed inputs produce identical bundle bytes. It does not claim
that separate native builds are reproducible.

Native source provenance is checked against the original committed Git tree,
the pinned Node/toolchain/ABI contract and every original build-input hash.
By default all native inputs must match the current checkout. The recorded
`receiptInputHash` is only the lookup identity for the existing native receipt
verifier; this assembler does not create or authenticate successful CI receipts.
Use that verifier and the original successful GitHub run/artifact identity when
obtaining the input artifact. A manifest and content hashes alone do not prove
where binaries came from or that they were built successfully.

For explicitly historical diagnostics, `assemble --allow-historical-native`
permits native source hashes that differ from the current native sources while
still checking their original commit and pinned runtime contract. Such a bundle
records `historical-source-only`, `receiptInputHash: null`, and
`unsigned-native-manifest-only`. It cannot count as an exact current native
build, an enrolled runtime, or a release artifact. This option is never inferred
from a failed receipt check.

Every result remains `unsigned`, not notarized, and not enrolled. Embedded Node
execution and containment are always `not-performed` in this assembly evidence.
`verify` detects inventory/content/contract inconsistencies, including a changed
library whose outer bundle hash was recomputed while the original native proof
was retained. An attacker can replace a complete unsigned bundle and recompute
all hashes: authenticity belongs to release signing and installer verification.
Platform signing changes bytes, so the final release inventory must be generated
after platform signing and notarization processing, then signed and enrolled
through the release contract. Never mutate this unsigned manifest into a claim
that those steps passed.

Both commands return one small JSON result on success; failures print only
`app_runtime_bundle_failed` and exit 1. Tests use tiny non-executable fake native
files and disposable Git fixtures outside the checkout:

```sh
node --test --test-concurrency=1 scripts/package-app-runtime.test.mjs
```

The separate lightweight bundle workflow tests inventory/tampering and the
native-versus-JS fingerprint distinction without triggering native compilation.
