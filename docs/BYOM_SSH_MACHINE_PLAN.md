# BYOM / Chariox Setup (MP-07 / MP-08 / MP-11)

PR1 and PR2 are rebased onto coordinator-requested OSS ownership `27aedc57e`
on main `60c1ccb1e`. Local protocol 479 is retained; relay 73 comes from that base. MP-11 follows the
owner's narrowed behavioural and security-anchor scope. Cloud remains control
plane; each independent kernel owns its runtime and provider execution.

## MP-07 / MP-08 Path-1 reuse

| Step | Existing code | BYOM |
| --- | --- | --- |
| VM provision | `runtime/managed_environment_control.rs`, Cloud `/managed-environments` | Omit provider provisioning/billing/deletion |
| Signed install | `scripts/package-managed-kernel-release.mjs`, `deploy/managed-kernel/{extract-release.py,verify-image-release.mjs}` | Reuse extractor and image signature/artifact/builder checks; replace root/system-service wiring |
| Identity/enrollment | `config/identity.rs`, `runtime/cloud_relay_login_executor.rs`; managed VM admission in `managed_bootstrap/` | Generate normal independent identity; use #888/#302 owner enrollment, keep managed VM admission unchanged |
| Service/relay | `managed_bootstrap/supervisor.rs`, `runtime/cloud_relay_{connection_executor,control}.rs` | Per-user systemd/launchd service; ordinary relay grants, registration and heartbeat |
| Project/context | `managed_context/{outbound_service.rs,kernel/import.rs,transfer/}`, `PrepareManagedEnvironmentContextTransfer` | Reuse encrypted export/import and approval in PR4; managed-environment-bound ticket needs independent-target adaptation |
| Provider accounts | `managed_context/kernel/{export.rs,import.rs}`, ordinary lease profiles | Owner update: no BYOM credential sync; login on target |
| Teardown | `RequestManagedEnvironmentLifecycle`, kernel quiescence | No VM deletion/auto-stop; stop or uninstall only the marked install |

## MP-07 / MP-08 / MP-11 shared installer

The source kernel runs the user's SSH config/agent, with host-key checking,
batch authentication and no agent forwarding. Chariox stores no SSH password
or key. PR1 supports Linux x86_64 with Python 3, Node 22 and systemd --user.
An operator-owned `ssh-machine-releases.json` beside source `config.toml`
selects the approved immutable release/version and independent public trust
pins; clients never supply arbitrary paths or pins. Selected Project context
is a separate optional transfer, not a copied kernel identity.

The source's own kernel credential issues a 600-second ticket via
`POST /v1/kernel-enrollment-tickets` and sends it over SSH stdin only. Target
`--owner-managed-enroll-stdin` redeems the #302 variant of `/auth/device/poll`:
`ticket`, `machineId`, `kernelId`, `publicKeyThumbprint`, optional string
`kernelAlias`; response is the ordinary approved profile + kernel credential.
No raw publicKey or null alias. Cloud account IDs and user IDs differ; compare
expected owner to userId, retaining shared machine/kernel/key validation.
Unused tickets revoke via DELETE. Hosted relay uses wss; bounded loopback mocks
are test-only. Ready requires authenticated local status matching enrolled
identity and a connected ordinary relay path.

Install ID selects distinct release root
`~/.local/share/chariox/ssh-machines/<id>`, private state
`~/.chariox/dev/ssh-machines/<id>`, service and loopback kernel/MCP port pair.
No sudo. Refuse foreign/edited units, symlinked ancestors and occupied/default
ports. Repeat retains identity and release. Failed readiness restores only service
start/enable changes made by that invocation; existing work stays running. Source reservations reconcile only
a positively absent target; uncertain or published installs stay protected.
Ticket-issue or invalid-selection failure cannot reserve a target.

PR2 generic Setup selects a signed distribution version/platform, verifies the
bundle and nested runtime inventory under an independent compiled public pin,
and uses the same marked installer core. Standalone device approval uses #888
kernel enrollment; CLI login enrolls the terminal separately and offers Setup
when no fresh local heartbeat exists, including isolated install roots.
`--enroll` accepts stdin or a hidden prompt only, never a code argument. No
per-user secret is embedded. Browser “Add this machine” is PR3's ticket UX.

Explicit repair restores only missing marked service files. Upgrade verifies
under original pins, activates atomically, checks unchanged ready identity,
and rolls back on failure; interrupted-journal recovery is tested. Forced death
can leave an exclusive lock requiring operator settlement. Uninstall stops only
this service/removes verified release bytes and links; private state and Cloud
identity remain. A validated, stopped/disabled install records removal intent
before unit unlink. Inspection retains its published ownership, and remove can
retry after reload failure; foreign units and active jobs remain refused.
CLI login resolves its marked install from the invoking release and forwards
that validated ID/port to Setup, including named and nondefault-port installs. Never adopt staging unit/root or delete the existing computer.

## MP-07 / MP-08 / MP-11 lean slices and acceptance

1. PR1: `/machine add ssh HOST` / `/machine remove ID`, source kernel request,
   signed SSH install, enrollment/readiness, local 479 snapshot/hash and actual
   localhost sshd + real-kernel drill against strict mock Cloud/relay.
2. PR2: generic Linux executable + unsigned .deb; macOS arm64 executable/app
   skeleton + launchd adapter; versioned signed `install.sh`, CLI/TUI login
   offer, lifecycle, unsigned CI artifacts and external owner signing hook.
3. PR3: Cloud generic downloads, browser code issue/revoke and one-liner UX.
4. PR4: Copy kernel/selected Project via existing export RuntimeInteraction and
   encrypted kernel transfer; provider login remains per machine.

See `apps/setup/README.md` and lane `LIVE_TEST.md` for commands/artifact contract.
Linux arm64/Windows, real macOS service/signing/notarization, signed public
publication, production Cloud and Mac-to-LAN acceptance remain coordinator
steps. No owner decision blocks source work. Private owner signing inputs stay
off builders. Local fixtures establish their named seams, not MP closure or
fresh-machine parity acceptance; Path-1 bootstrap checks are unchanged.

## MP-07 / MP-08 / MP-11 ownership rebase

The strict startup parser admits SSH, self-ticket and device stdin enrollment
and readiness as distinct typed commands, rejecting extra arguments before
runtime initialization. Inherited Browser/Computer/public-provider guards bind
local 479 and retain relay 73; the aggregate version/hash is reconciled. Local
source/mock drills do not establish live Cloud, signed distribution, real user
services or fresh-machine acceptance.

## MP-08 / MP-11 credential-free export boundary and threat model

Owner-managed copy uses the ordinary kernel export/import path. A source owner
may accidentally select credential-bearing context; an untrusted source may
compose a correctly hashed snapshot to bypass source admission. The target
rechecks the same boundary before publication. Hashes establish content identity,
not the absence of secrets. This policy does not alter credential-bearing
Path-1 transfer, its explicit Vault binding, or official provider profiles.

The exportable structured schema is the typed Extension/dependency model.
MCP definitions allow transport command/arguments or URL, working directory,
names, tool policy, enablement and numeric timeouts. Credential-free source
projection omits **all** literal environment/header maps, ambient environment
bindings and credential bindings; no field-name guess decides whether a value
is safe. The target refuses snapshots carrying any of those slots. Users must
configure required values on the receiving kernel through its normal Vault
and capability settings. This copy mode never exports provider/workspace
settings, provider profiles, credential registry, Vault or Project environment
value layers as structured dependencies. It does not read them to redact them.

Skill metadata exports name, descriptions and portable path; packaged skill
files remain owner-selected free-form content. Script metadata allows runtime,
description, numeric timeout and the supported JSON Schema vocabulary. Schema
property/definition names describe inputs; arbitrary defaults, constants,
examples, enum values and unknown schema extensions are refused. Connector
operation settings allow only string `url`/`method`/`path` and unsigned numeric
`timeout_ms`/`max_response_bytes`; other arbitrary settings are refused. This
closed allowlist deliberately refuses unsupported configuration rather than
classifying it by whether a key sounds secret. New typed fields/variants require
an explicit export decision.

Every packaged file is scanned under its actual path, including skill/MCP,
portable environment and user adapter files. Standalone script source retains
Python/TypeScript runtime context. The same file detector applies to development
overlays and Git history: high-signal private-key/provider-token formats,
credential assignments/flags/URLs, and strict UTF-8 or BOM-marked UTF-16/32
decoding. Unsupported bytes, malformed encodings and binary controls fail
closed. Shell paths and shell shebangs also refuse undecodable shell quoting.
Dependency-name maps and lock integrity have explicit package metadata roles;
their values remain inspected. Literal structured manifest copies occurring
inside owner-selected files are free-form inputs, subject to the same file
guard and owner review, never a way to populate the typed configuration schema.

**The scanner is a guard, not a proof.** Arbitrary secrets may look like normal
prose, source literals, allowed URLs or metadata. Computed values, arbitrary
encodings/encryption, external references and every language's execution
semantics are outside its finite detectors. It neither executes files nor
certifies that a package is safe to execute. False refusals are possible,
including secret-looking examples, unsupported schema features and binary
assets. The owner must review and confirm the selected exports in the source
Project RuntimeInteraction, including packaged files and repository history;
that confirmation does not bypass a scanner refusal. Target admission, owner
and plan binding, integrity checks, publication ownership and recovery remain
independent controls. No whole MP-10 matrix or MP-11 review gate closes from a
scanner test or this bounded two-kernel drill.
