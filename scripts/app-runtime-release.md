# Signed Linux App runtime assembly

`sign-app-runtime-release.mjs` assembles the exact runtime graph accepted by
`runtime_enrollment`. It requires a separately trusted builder signature and a
release signing key; an unsigned native receipt or a key supplied inside the
runtime directory cannot authorize a release.

The input directory contains `bundle/`, produced by `package-app-runtime.mjs`,
and `native/`, containing the native worker launcher, domain entry, pinned
bubblewrap and eight fixed platform libraries. The target selects the loader:
`platform/ld-linux-x86-64.so.2` or `platform/ld-linux-aarch64.so.1`. Both targets
also contain `platform/{libc.so.6,libm.so.6,libstdc++.so.6,libgcc_s.so.1,
libpthread.so.0,libdl.so.2,librt.so.1}`. Worker startup uses this signed graph;
it must not discover a different guest library graph from the host filesystem.
Host dependencies of the trusted pre-namespace launcher remain an installer
responsibility; this guest graph does not certify those host dependencies.

The builder's exact UTF-8 JSON bytes use this contract:

```json
{
  "schema": "chariox.app-runtime-release-build.v1",
  "target": "linux-x64",
  "sourceCommit": "<40 lowercase hex>",
  "bundleDigest": "sha256:<bundle manifest digest>",
  "launcherBuildInputs": [
    {"path": "<exact launcher source path>", "sha256": "<64 lowercase hex>"}
  ],
  "files": [
    {"path": "<final relative path>", "size": 123, "sha256": "<64 lowercase hex>", "executable": false}
  ]
}
```

`files` is the complete lexically sorted final inventory: every bundle file,
its manifest, the three native executables and the eight platform libraries.
Only those executables and the platform loader (the ELF interpreter of every
worker executable, which Linux opens for execution) have `executable: true`. The detached builder signature
is exactly 128 lowercase hexadecimal characters without a trailing newline.
Builder and release keys are Ed25519 PEM files supplied outside the input tree.
The private release key must be owned by the invoking user with no group or
other permissions. The release process exclusively owns its working directories.
`launcherBuildInputs` is the sorted exact `LAUNCHER_INPUTS` set in the release
contract, including the sandbox pin, launcher sources/headers and domain entry.
It is separate from the expensive Node cache inputs. Both sets must match their
committed and current source bytes before a runtime can be released.

```sh
node scripts/sign-app-runtime-release.mjs \
  --input /absolute/build-input \
  --builder-attestation /absolute/builder.json \
  --builder-signature /absolute/builder.sig \
  --trusted-builder-key /absolute/trusted-builder.pem \
  --signing-key /absolute/private-release.pem \
  --output /absolute/new-release-directory
```

The tool checks the external signature, committed/current source identities,
exact inventories and content digests before signing. It rejects historical
native evidence, extra files, links, incomplete platform graphs and substituted
bytes. It streams copies within the 512 MiB total/40-file limits. All payload
files are mode 0444 or 0555, and output includes `runtime-inventory.json`, its
detached signature and an empty `.runtime-lease`. The returned digest and public
key identify the result; they are not installed trust. No key or self-enrollment
file is copied into the result.

## macOS releases: codesign first, then the inventory

Code signing changes Mach-O bytes, so the Chariox inventory must describe the
codesigned files. A darwin input holds `bundle/` and `native/chariox-app-worker`
only. The order is:

1. The builder attests the unsigned input, as above.
2. The owner codesigns and notarizes a copy of the whole input directory with
   `sign-macos-release.mjs` (see "macOS Developer ID signing"). The copy keeps
   the input's layout.
3. The owner signs the inventory over that copy:

   ```sh
   node scripts/sign-app-runtime-release.mjs --input /absolute/build-input \
     --codesigned /absolute/signed --codesign-identity "$CHARIOX_CODESIGN_IDENTITY" \
     --builder-attestation /absolute/builder.json --builder-signature /absolute/builder.sig \
     --trusted-builder-key /absolute/trusted-builder.pem \
     --signing-key /absolute/private-release.pem --output /absolute/new-release-directory
   ```

4. The root installer enrolls the printed key and digest, as on Linux.

Step 3 verifies the unsigned input exactly as on Linux. It then requires each
signed `chariox-app-worker`, `libnode.137.dylib` and
`libchariox-app-runtime.dylib` to equal its attested unsigned bytes everywhere
outside the embedded code signature: only the signature, its load command and
the `__LINKEDIT` sizes that hold it may differ. Every other file must be the
attested bytes. On the copies it signs, codesign must accept each signature
with the hardened runtime; the worker carries only `allow-jit`, and the libraries
carry no entitlements. A production release also requires the named Developer
ID identity, a secure timestamp, Apple's Developer ID requirement for that team
and a Gatekeeper `Notarized Developer ID` verdict on the worker. Only then is the
inventory signed, with the signed sizes and digests. `bundle-manifest.json`
keeps the builder's unsigned digests as provenance. Without `--codesigned`,
a darwin release is refused.

The developer path, `app-runtime-local-release.mjs`, can codesign the same way
with `--codesign-identity -` (ad hoc) or a local identity, without notarization.
It then checks everything above except the Developer ID facts. Without that flag
it still signs its linker-signed files unchanged.

The system installer must independently authorize the release key and digest,
copy the graph into a root-owned version directory with traversable immutable
directories, then atomically publish the separate fixed enrollment. The worker
retains the enrolled generation's shared lease through process and broker drain;
cleanup requires the exclusive lease. The assembler does not run the artifacts,
install authority or claim sandbox/embedded execution validation. Production
builder attestation generation and the root enrollment installer remain
integration work.

## macOS Developer ID signing

`sign-macos-release.mjs` Developer ID-signs and notarizes macOS release code:
the kernel, its helpers, and the App runtime's worker and libraries. It runs
on the owner's Mac. The owner names the identity and the notarytool keychain
profile at run time. The tool never creates, stores or reads a credential,
certificate or profile.

It copies `--input` to a new `--output` directory and leaves the input
unchanged. Each Mach-O file is classified by its header: `*.dylib` files are
libraries, everything else is an executable, and other files are copied
unchanged. Libraries are signed before executables. Every Mach-O file gets a
Developer ID signature with `--options runtime` and `--timestamp`.
Only `chariox-app-worker`, the process that runs V8, also gets
`macos-app-worker.entitlements`, which grants `com.apple.security.cs.allow-jit`
and nothing else, so library validation stays on.

The tool then checks each signature with `codesign --verify --strict`, and
checks that each file names the identity and team, has a secure timestamp and
the hardened runtime, and has exactly the expected entitlements. It zips the
output, submits it with `notarytool submit --wait`, and keeps the notary log.
Finally, it asks Gatekeeper (`spctl --assess --type install`) to accept every
executable as notarized Developer ID code.

Bare Mach-O files and zip archives can't hold a stapled ticket; `stapler` only
accepts app bundles, disk images and flat packages. So the tool records
stapling as not applicable, and Gatekeeper checks the ticket online.

If any step fails, the tool removes the output directory and the archive. On
success, it prints a JSON receipt with the unsigned and signed digests of
every file, the notarization submission ID, and the Gatekeeper results. It
refuses an input that contains `runtime-inventory.json`,
`runtime-inventory.sig` or `.runtime-lease`, because a Chariox runtime
inventory must be signed over the platform-signed bytes, after this step.

Owner prerequisites, once:

1. Join the Apple Developer Program. Create a **Developer ID Application**
   certificate, either in Xcode (Settings › Accounts › Manage Certificates)
   or at developer.apple.com › Certificates. Back up the certificate and its
   private key as a password-protected `.p12`, and keep the backup off-device.
2. Confirm the identity: `security find-identity -v -p codesigning` lists
   `Developer ID Application: <Name> (<TEAMID>)`.
3. Store the notary credentials in your keychain. The command prompts for an
   app-specific password from appleid.apple.com:
   `xcrun notarytool store-credentials chariox-notary --apple-id <Apple ID> --team-id <TEAMID>`.
   An App Store Connect API key also works:
   `--key <AuthKey.p8> --key-id <id> --issuer <uuid>`.

Each release:

```sh
export CHARIOX_CODESIGN_IDENTITY='Developer ID Application: <Name> (<TEAMID>)'
export CHARIOX_NOTARY_PROFILE=chariox-notary
node scripts/sign-macos-release.mjs --dry-run \
  --input /absolute/unsigned --output /absolute/signed
node scripts/sign-macos-release.mjs \
  --input /absolute/unsigned --output /absolute/signed > signing-receipt.json
```

The dry run lists every file with its role and digest, and prints every
command without running any of them. Its output is the review step.
`--identity` and `--keychain-profile` override the environment variables.
macOS asks before `codesign` uses the private key; answer **Allow**, not
**Always Allow**. If notarization is rejected, the error prints the
`xcrun notarytool log <id> --keychain-profile <profile>` command to run.
