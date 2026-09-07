# Chariox App package contract v1

`chariox-app-package` supplies an offline Rust packer, an explicitly untrusted
identity inspector, and a verifier. It does not install or execute Apps, enroll
publisher keys, fetch packages, run npm, or implement a sandbox. The kernel
installer must supply enrolled publisher keys and compatibility policy, then
consume only a `VerifiedPackage` when staging files.

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
