# Chariox App package contract v1

`chariox-app-package` supplies an offline Rust packer, an explicitly untrusted
identity inspector, and a verifier. It does not install or execute Apps, enroll
publisher keys, fetch packages, run npm, or implement a sandbox. The kernel
installer must supply enrolled publisher keys and compatibility policy, then
consume only a `VerifiedPackage` when staging files.

## Local developer commands

The shared CLI supports `chariox app create`, `keygen`, `manifest`, `pack`,
`validate`, and `inspect` on macOS/Linux. Both the Rust launcher and direct
TypeScript CLI dispatch to the same `chariox-app-package` executable. Developer
commands preserve the caller's working directory, do not connect to a kernel,
and never start an App, install dependencies or run package scripts.

Build the small helper explicitly when working from source:

```sh
cargo build --locked -p chariox-app-package --bin chariox-app-package
```

The Rust launcher locates the helper beside its own executable, or at the
managed installation's `/usr/local/bin/chariox-app-package`. The TypeScript CLI
also recognizes its own checkout's `CARGO_TARGET_DIR` and `target` outputs.
`CHARIOX_APP_PACKAGE_BIN` selects an absolute executable explicitly. Neither path
searches the current directory or `PATH`, or starts Cargo automatically. The
managed Linux release includes the helper in its signed build attestation and
release inventory; its installer verifies and exposes that exact executable.
This does not imply that the managed image ships the full TUI. A separate
native client installer is not implemented here.

Generate a developer identity once, outside your project, then create an App:

```sh
mkdir -p "$HOME/.chariox/dev"
mkdir -m 700 "$HOME/.chariox/dev/app-publisher"
chariox app keygen --publisher-id com.example --publisher-name Developer \
  --key-out "$HOME/.chariox/dev/app-publisher/private" \
  --trust-out "$HOME/.chariox/dev/app-publisher/publisher.json"
chariox app create my-app --app-id com.example.greeting \
  --publisher "$HOME/.chariox/dev/app-publisher/publisher.json"
chariox app pack --bundle my-app/bundle --manifest my-app/app.json \
  --key "$HOME/.chariox/dev/app-publisher/private" --output my-app.cxapp
chariox app inspect my-app.cxapp
chariox app validate my-app.cxapp --trust my-app/publisher.json
```

`create` currently requires an existing public publisher file. It creates
`app.json`, a public `publisher.json`, an ESM `greet` handler and matching closed
schemas, a basic accessible HTML view, `.gitignore`, and a short README. Version
defaults to `0.1.0`; `--version` selects another version. It generates or copies no
private key and embeds no machine-specific paths in the project. Its destination
must not exist, including an empty directory or symlink. Validation precedes
creation; an I/O error can leave an incomplete new scaffold and never overwrites
or removes user files. Every write uses anchored directory descriptors.

The shared CLI supplies its actual protocol constant for create/manifest/pack/
validate; it rejects a user override. The standalone tool requires
`--kernel-protocol N` explicitly for callers selecting another compatibility
contract. This number is not a claim that the complete App runtime is available.
`manifest` also supplies the supported SDK/App/resource contracts and accepts
optional runtime/UI entries, declarations and repeated `--network METHODS=ORIGIN`.
Capabilities start empty; declaring one does not grant permission. A richer
capability editor remains separate work.

Keep the manifest, signing key and package output outside the input `bundle/`
directory. Paths may be absolute or relative to the original caller directory;
parent components (`..`), symlinks and special files are rejected. Use an
absolute path for an external key when packaging from inside your project.

`keygen` uses OS randomness. The private file contains exactly 32 raw Ed25519
seed bytes, is created with mode `0600`, and requires a current-user private
parent directory (`0700`). Its seed is never printed or included in JSON results.
The public file is `chariox.developer-publisher.v1` enrollment material; supplying
it to `validate` establishes trust only for that command. The kernel must still
enroll a publisher through its own explicit path. Generated key IDs bind the
public-key fingerprint, and `pack` rejects a different private key.

Existing outputs are preserved. Each output is written to a private temporary
inode, synced, then atomically renamed without replacement using the supported
OS's exclusive-rename operation. Key generation
prepares both files before publishing the private key and then the public file.
Two output paths are not one filesystem transaction: a crash can leave a valid
private key without enrollment material. An enrollment-publication failure
reports that condition; it never overwrites another key or silently enrolls
anything. Output parents must belong to the current user and cannot be writable
by group or others. Use a filesystem supporting exclusive rename and directory
synchronization. The writer retains its temporary inode and checks its identity
before publication and cleanup.

Bundle traversal opens each component relative to an already opened directory
and rejects symlinks, hard-linked files, devices, FIFOs, parent traversal,
nonportable names, and declared bounds. Empty directories count against the
traversal bound. File bytes are read into bounded buffers before signing;
detected concurrent changes fail. Freeze build output before packaging: this
tool does not supply a transaction across concurrently changing input files.
Packing holds the selected bundle, signing-key, and output-parent descriptors;
renaming/replacing their pathnames does not change those selections. A signing
key inode moved into the input during traversal is rejected before its bytes
are read into the bundle.
The completed archive passes the shared verifier before publication. Signing
keys and outputs cannot sit inside their own input bundle, including through
macOS case aliases.

Every standalone/Rust-launcher CLI result is one JSON record on stdout: `{ "ok": true, "result": ... }`
or `{ "ok": false, "error": { "code": ..., "message": ... } }`. Exit codes are
`0` success, `2` invalid arguments, `3` rejected content/key/compatibility, and `4`
filesystem/output failure. The direct TypeScript CLI prints the successful result
object and treats helper failure as command failure. `inspect` always reports `untrusted-claims-only`;
it does not authenticate signatures or payloads. `validate` reports
`verified-against-explicit-publisher-file`, never installed or activated.

The integration tests exercise the real binary, deterministic packing, explicit
trust, tampering, key privacy, nonreplacement, link/special-file handling,
bounds, and generation of a manifest without executing bundled code. Run them
with `cargo test -p chariox-app-package`; no Node build or App execution occurs.

## Archive encoding

A `.cxapp` is an **uncompressed USTAR archive**. There is no ZIP, gzip, PAX,
GNU extension, sparse file, link, directory, device, or executable entry type.
Directory creation is an installer responsibility after verification.

Files occur in this exact order:

1. `manifest.json`
2. `integrity.json`
3. `signatures/publisher.sig`
4. Payload files, ordered by ASCII path bytes.

The canonical USTAR header is produced by `tar::Header::new_ustar()` with the
path, regular entry type `0`, mode `0644`, uid/gid/mtime zero, the exact size,
and the computed checksum. Unused fields are zero. USTAR name/prefix splitting
uses the `tar` crate's path representation: paths up to 100 bytes use `name`;
otherwise the longest parent prefix of at most 155 bytes is chosen, and the
remaining basename portion must fit in 100 bytes. Payload padding is zero and
the archive ends in exactly two 512-byte zero blocks. The verifier regenerates
every header and compares all 512 bytes, including checksum and unused fields.

Package filenames use portable printable ASCII. Content can contain any Unicode
or binary data. Absolute paths, empty/dot/parent components, backslashes, drive
names, control characters, Windows-special characters/device names, trailing
dots/spaces, duplicate files, file/directory conflicts, and case collisions
(including parent directories) are rejected. Non-ASCII names must be renamed by
the bundle build; rejecting them removes filesystem Unicode-normalization
ambiguity. USTAR path limits apply in addition to configured limits.

Default bounds: archive 128 MiB; file 64 MiB; control/declaration document 1 MiB;
individual schema 128 KiB; 4,096 archive entries including control files;
255-byte paths; path depth 24; JSON depth 32 and 32,768 nodes per document;
256 declarations per category. Callers may lower these limits. Compression
bombs cannot be expanded because no compressed encoding is accepted. The
verifier borrows file contents from the bounded input buffer.

## Signature envelope

All three control files use RFC 8785 canonical JSON, without a trailing newline.
All JSON documents reject duplicate object keys and out-of-range interoperable
integers. Declaration files may use ordinary whitespace/key ordering: their
exact bytes are covered by the signed inventory.

`integrity.json` has this shape:

```json
{
  "schema": "chariox.integrity.v1",
  "files": [
    {"path": "runtime/main.js", "size": 18, "digest": "sha256:<64 lowercase hex digits>"}
  ]
}
```

The inventory lists every payload file exactly once in archive order. It
excludes `manifest.json`, `integrity.json`, and the entire `signatures/` tree.
The manifest is separately included in the signed object; the inventory cannot
include its own digest. No other signature files are accepted in v1.

Ed25519 signs the RFC 8785 bytes of exactly this object:

```json
{
  "schema": "chariox.package-signature.v1",
  "manifest": {"...": "the complete manifest object"},
  "inventory": {"...": "the complete integrity object"}
}
```

`signatures/publisher.sig` uses the existing Chariox signature field names:

```json
{
  "key_id": "developer-1",
  "algorithm": "ed25519",
  "digest": "sha256:<digest of the canonical signed object>",
  "value": "<standard padded base64 of the 64-byte signature>"
}
```

The signature key ID must equal `manifest.publisher.keyId`. Trust lookup binds
both publisher ID and key ID to an externally enrolled Ed25519 public key;
neither a publisher claim nor a package-provided key grants trust. Verification
uses strict Ed25519 verification. The full-package digest is separately computed
over every archive byte for installation/release identity.

## Manifest and declarations

See the exported types in `src/manifest.rs` and `src/declarations.rs`. Unknown
fields are rejected. This is a minimal valid manifest (the kernel protocol
number is illustrative; the selected SDK/runtime must supply its actual floor):

```json
{
  "schema": "chariox.app.v1",
  "appId": "com.example.todo",
  "version": "1.0.0",
  "publisher": {"id": "com.example", "keyId": "developer-1", "name": "Local Developer"},
  "sdkVersion": "0.1.0",
  "appContractVersion": 1,
  "minKernelProtocol": 500,
  "resourcePolicy": "chariox.app.resources.v1",
  "runtime": {"engine": "node", "entry": "runtime/main.js"},
  "ui": {"entry": "ui/index.html"},
  "capabilities": {}
}
```

The packer receives a typed manifest and an already built payload map. Its
canonical serialization fills defaults. The verifier signs/checks the original
canonical manifest object so optional defaults cannot change signed meaning.
SDK compatibility, App contract version, kernel protocol floor, and resource
policy support are separate kernel-supplied checks. The default SDK requirement
is exactly `0.1.0`; it is not a claim of compatibility with future SDK releases.

Optional `tools`, `events`, `actions`, and `informationSets` properties refer to
JSON files within `schemas/`. These files contain respectively:

- `{"tools": [{"name", "description"?, "inputSchema", "outputSchema"?, "action"?}]}`
- `{"events": [{"name", "payloadSchema", "filterSchema"?}]}`
- `{"actions": [{"name", "inputSchema", "criticalValidation"?, "effectRoutes"?}]}`
- `{"informationSets": [{"name", "purpose", "sourceScope", "schemaVersion", "delivery", "fieldsSchema", "validator"?}]}`

The compact shapes above denote fields, not literal JSON examples. Local
function names are lowercase letters/digits/underscores, start with a letter,
and are unique within each category. The kernel supplies the installation/tool
namespace. JSON schemas are compiled as Draft 7. Inputs, event payloads/filters,
and information fields require `type: "object"` and
`additionalProperties: false`. Schemas must be inline, without `$id`, `$ref`,
`$dynamicRef`, or `$recursiveRef`; the build must resolve reusable references
before packing. No external schema retrieval occurs. App validators are local
function references; they never execute during package validation.

The ID/reference rule applies at actual schema positions, not to property names
or literal data in defaults, `enum`, or `const`. An explicit deny-all resolver
keeps compilation offline even when workspace dependencies enable network
features. `compile_schema()` supplies the same bounded compiler for kernel
runtime checks; a compile-only projection prevents upstream schema-ID discovery
from interpreting arbitrary annotations as references. The original signed
declarations, defaults, and domain annotations are retained.

Network capabilities declare exact canonical HTTP(S) origins and a nonempty
set of allowed methods. No wildcard, userinfo, path, query, or fragment is
allowed in an origin. `clipboard`, `externalFiles`, `workflows`, and `agents`
use the exported closed capability enums. These are declarations only: the
kernel still applies existing permissions, explicit data-sharing consent,
protected-effect approval, destination checks, and credential policy.

A critical action declares a reason, whether user verification is required,
and one or more effect routes. Each route is an exact origin/method/path and
symbolic connection class within declared network capabilities. Routes cannot
overlap another declared action. Tools referencing an action use the same input
schema. The kernel must intercept the declared route at execution; package
validation alone cannot prove that arbitrary service requests represent a
particular business effect.

Information sets scope collection to `app_task` or `agent_turn`, declare a
positive schema version and `intermediate`, `final`, or `both` delivery, and
optionally reference a sandboxed App validator. Declaring a set never grants
consent or transcript access.

Optional migrations declare `directory: "migrations"`, `targetVersion`, and a
complete ordered chain `steps: [{from: 0, to: 1, entry: "migrations/001.js"}, …]`.
Every step advances one schema version, has a unique existing JavaScript file,
and there are no undeclared migration files. Execution, generation fencing,
transaction boundaries and recovery belong to the kernel lifecycle service.

## Verification boundary and tests

`inspect_untrusted()` returns only a claimed identity and archive digest for
trust UI. `verify()` checks archive shape, control JSON, enrolled-key signature,
compatibility, exact inventory correspondence, every payload digest, entry
references and declaration contracts before producing `VerifiedPackage`.
Its file fields cannot be constructed externally. The installer must stage
these verified bytes into its own fresh private directory without following
pre-existing paths or links; do not verify a filename then reopen mutable bytes
for extraction. Browser isolation and the OS App sandbox remain required for
all executed code, including code evaluated or downloaded after installation.

Run `cargo test -p chariox-app-package`. The corpus includes reproducibility,
an independent signed fixture writer, standard USTAR readability, wrong trust
identities/keys, signature and payload tampering, missing/extra files,
compatibility categories, migration graphs, invalid/recursive/external schemas,
duplicate JSON keys, protected routes, information-set scope, hostile header
types, traversal, case/Unicode handling, limits, truncation, padding, compressed
input, and canonical control encoding. These tests do not establish installer
or App runtime conformance.
