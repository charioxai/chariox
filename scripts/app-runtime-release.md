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
Only those executables have `executable: true`. The detached builder signature
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

The system installer must independently authorize the release key and digest,
copy the graph into a root-owned version directory with traversable immutable
directories, then atomically publish the separate fixed enrollment. The worker
retains the enrolled generation's shared lease through process and broker drain;
cleanup requires the exclusive lease. The assembler does not run the artifacts,
install authority or claim sandbox/embedded execution validation. Production
builder attestation generation, the root enrollment installer and the macOS
Developer ID/notarization release path remain integration work.
