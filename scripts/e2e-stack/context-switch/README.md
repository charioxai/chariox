# MP-08 / MP-10 / MP-11 context-switch drills

Supplementary tooling promoted from ctxfix2 r10. Source-only tests verify packaging,
opaque network shaping, recall scoring and brief observers. They **do not establish
live acceptance**. Run user-facing cells using built Chariox TUI, production Cloud
entry, hosted WSS, real official provider profiles, ordinary clicks/keys, and retained
screenshots/video. Inspect recordings before classifying UI PASS. Native Claude
cancel remains a separate #993 cell. Slice/Path-1 placement is a separate row.

From a fresh checkout on Linux with Node 22 and Python 3 (no installed npm packages or credentials needed):

```sh
node scripts/e2e-stack/context-switch/self-test.mjs
```

The self-test checks the local import/worker closure, executes the actual shaped
proxy worker, and tests nine-fact scoring plus watermarks. Deleting
`shaped-proxy-worker.mjs` must fail. Loopback byte fixtures are packaging regressions.

Live inputs are explicit and stay outside every repository:

- `CTXVAL_OWNER`: lane resource label (set `ctxfix2` on b3); default context-switch.
- `CTXVAL_TREE`: exact checked-out runtime source, default this checkout.
- `CTXVAL_LANE`: external build assets (`bin/`, `client/`, optional `ns-tools/`).
- `CTXVAL_STATE_ROOT`: disposable external runtime parent; no default Vault is used.
- `CTXVAL_EVIDENCE`: external screenshots, recordings, hashes and allowlisted logs.
- `PLAYWRIGHT_MODULE`: absolute installed Playwright Core entry.
- `CTXVAL_ORIGIN`: own provisioned production Cloud stack, with hosted WSS.
- `CTXVAL_NATIVE_CODEX_PROFILE`: disposable official native scope prepared by the
  approved product import; never manually refresh a shared login.
- `CTXVAL_OPENCODE_PROFILE`: product-linked scope, for OpenCode-specific cells only.
- `CHARIOX_CLAUDE_BIN`: installed official Claude CLI, also required for guest assets.

Pass the existing coordinator Claude setup token on **stdin**, never as an argument
or output. `bin/kernel-<label>`, `bin/relay-<label>`, `bin/chariox` and built
`client/apps/cli/dist/` must come from the exact source head. `bin/ready-<label>`
contains its 40-character head. `bin/build-<label>.json` has public provenance:

```json
{"head":"<40-character head>","protocol":491,"client":{"head":"<same head>"},"sha256":{"kernel-921":"<sha256>","relay-921":"<sha256>","chariox":"<sha256>"}}
```

Use the **actual** protocol; 491 above is an example, not a pin. Live startup checks
head/marker and all three binary hashes. Build through the shared cargo-slot wrapper;
touch sources within the admitted slot before compiling and capture artifacts while
holding it. Compile the actual CLI and shared client from that same tree. No copied
binaries, dependency trees, logs or homes belong in Git.

Non-model cells use Codex gpt-6-luna low and Claude claude-haiku-5-5.
Account commands use fresh registered display labels; acknowledgements still
check immutable profile IDs, including both legs of an account round trip.
Explicit window/model-change cells may use Sonnet. OpenCode uses
opencode-go/deepseek-v4.1-flash with documented refusal fallbacks; the driver rejects
more than three turns in an OpenCode session. Never use OpenCode in a soak.
Remote TUI shaping uses owned namespaces and external `ns-tools/` slirp4netns.
The viewer network helper uses the opaque CONNECT worker; it never terminates TLS or prints headers.
Turn oracles wait for five seconds of idle, unchanged assistant text and score
the same fresh completion snapshot retained as evidence. An end marker alone
can arrive before the final client history patch.

The former 150ms gate is measurement
only under owner C2. Runtime requests used for enrollment/read-only observation do
not replace actual TUI/Web input for user-facing acceptance.

For leased KVM cells set `CTXVAL_LEASED=1`, `CTXVAL_VM_ASSETS` (base.img),
`CTXVAL_VM_IMAGE` (existing qemu/cloud-localds tools image), and `CTXVAL_VM_SLOT=1`
or `2`. Guest input preparation is reproducible:

```sh
node scripts/e2e-stack/context-switch/prepare-worker.mjs
```

It exports public source/build/client inputs into `CTXVAL_LANE/public`; credentials
are enrolled/transferred later by the normal product paths. The diagnostic bridge
is guest stdio/control observation only; provider execution still uses home-owned
leases over hosted WSS. VMs admit only above 15% free disk / 24 GiB available memory,
use four vCPUs / 6 GiB, and reserve the shared b3 VM slot. Ordinary Docker short-cell mode
requires `CTXVAL_WORKER_KIND=docker` and `CTXVAL_DOCKER_IMAGE`; `CTXVAL_PEER_ALLOW_HELPER` optionally names the operator peer-admission helper;
only a unique exact own note is removed at cleanup. `CTXVAL_NODE_BIN` selects a
verified portable Node executable for a guest with another OS. Same-host mode is
supplementary and cannot establish real-machine acceptance.

Every run has a unique safe name; existing state ownership markers reject reuse.
The production signal guard records process generations and rejects unsafe PIDs or
foreign groups. Cleanup stops only captured own processes, deletes receiving
state/guest overlay/seed/slot, and records remaining processes. Stop own PR stacks
and remove KEEP when the round ends. Delete public guest assets/copied binaries
then; preserve evidence and shared provider profiles. No CI or publication runs.

MP-08 / MP-10 / MP-11: worker startup also waits for its exact machine/kernel
pair to appear ONLINE in the real hosted dashboard. A connected socket alone
does not establish a fresh selectable target. Authorization failures after that
check remain failures; the driver does not substitute credentials or bypass
admission.

MP-08 / MP-10 / MP-11: state roots are created exclusively; an existing directory
is never adopted. Cleanup also checks exact disposable home/profile paths for
detached account-inspection servers, with captured process generations and the
production signal guard. It records those removals separately and preserves
other runtime scopes. This is drill cleanup, not a claim that production kernel
shutdown has been repaired.

MP-08 / MP-10 / MP-11 interruption uses the shared termination registry before
any detached kernel, namespace, native client or worker starts. SIGINT/SIGTERM
settle owned resources before exiting. Run the supplementary process and oracle
regressions with an external disk-backed TMPDIR:

```sh
node --test scripts/e2e-stack/context-switch/termination.test.mjs scripts/e2e-stack/context-switch/control-validation.test.mjs
```

The source-outage oracle requires the source run/model's timeout record and
approximately 300 seconds of kernel elapsed time. Account transitions require
fresh native IDs on both legs; profile labels and recall alone cannot pass.
The cancellation cell checks a fresh snapshot after its eight-second observation
window and again after recovery, scoped to the cancelled agent. A late answer in
either interval fails the cell; a missing agent pane cannot establish absence.
The receiving profile in `account-read-live.mjs` uses product linking/refresh; a fresh 401 is recorded as
a coordinator account action, while primary-thread continuity is checked.

MP-08 / MP-10 / MP-11: `draft-config-live.mjs` replays the real ANSI editor with Python `pyte`; set `CTXVAL_PYTHON` to an interpreter with that dependency installed. The general screenshots use Python Pillow. Provider account commands take the public profile label, while RPCs use stable profile IDs.

MP-08 / MP-10 / MP-11: `handoff-config-live.mjs <build-label> <run-name>`
is the focused local527 wire drill. It needs matching built client/kernel/relay
assets, a hosted origin, and `CTXVAL_NATIVE_CODEX_PROFILE` for normal product
account import/status checks before TUI admission. It sends zero provider turns.
It writes both
brief-model keys using the real TUI over the shaped hosted relay, checks populated
`UserConfig` / `UserConfigUpdated` values and their canonical handoff hash, verifies
client and kernel advertisements, then restarts and checks persistence. The base
491 build must fail the advertisement assertion after successfully setting values.
This proves the config seam only; it does not classify startup, latency, provider
handoff, multi-hour or Path-1 acceptance.

MP-08 / MP-10 / MP-11: the standalone self-test gives child tests an owned temporary
directory under `TMPDIR`, or `~/.chariox/dev/context-switch/tmp` when unset, and
removes it after the run. Repository-local temporary roots are rejected.
