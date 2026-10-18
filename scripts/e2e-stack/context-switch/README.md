# MP-08 / MP-10 / MP-11 context-switch drills

Supplementary tooling promoted from ctxfix2 r10. Source-only tests verify packaging,
opaque network shaping, recall scoring and brief observers. They **do not establish
live acceptance**. Run user-facing cells using built Chariox TUI, production Cloud
entry, hosted WSS, real official provider profiles, ordinary clicks/keys, and retained
screenshots/video. Inspect recordings before classifying UI PASS. Native Claude
cancel remains a separate #993 cell. Slice/Path-1 placement is a separate row.

From any fresh source checkout (Node 22, no dependencies or credentials needed):

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

Example after preparing exact external assets and an own hosted stack:

```sh
node scripts/e2e-stack/context-switch/claude-pairs.mjs 921 local-pairs
node scripts/e2e-stack/context-switch/drill.mjs 921 local-faults faults
node scripts/e2e-stack/context-switch/default-account.mjs 921 compaction
```

`CTXVAL_PAIRS=codex-claude-p` narrows pairs; `CTXVAL_SOURCE_TURNS` bounds source audit
work. Non-model cells use Codex gpt-6-luna low and Claude claude-haiku-5-5.
Explicit window/model-change cells may use Sonnet. OpenCode uses
opencode-go/deepseek-v4.1-flash with documented refusal fallbacks; the driver rejects
more than three turns in an OpenCode session. Never use OpenCode in a soak.
`default-account.mjs` is the historical compaction driver: its 160k threshold alone
does not prove current `/context`, >75%, queue-once, negative controls or outage cells.

Remote TUI shaping uses owned namespaces and external `ns-tools/` slirp4netns.
Web uses the opaque CONNECT worker; it never terminates TLS or prints headers.
Web records screen video and screenshots at DPR1/2, default 1440×900. Repeat
with `CTXVAL_VIEWPORT='{"width":1920,"height":1080}'` for the second desktop case;
inspect every recording for stuck or blank views. The former 150ms gate is measurement
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
use four vCPUs / 6 GiB, and reserve the shared b3 VM slot. Docker diagnostic mode
requires `CTXVAL_WORKER_KIND=docker` and `CTXVAL_DOCKER_IMAGE`; same-host mode is
supplementary and cannot establish real-machine acceptance.

Every run has a unique safe name; existing state ownership markers reject reuse.
The production signal guard records process generations and rejects unsafe PIDs or
foreign groups. Cleanup stops only captured own processes, deletes receiving
state/guest overlay/seed/slot, and records remaining processes. Stop own PR stacks
and remove KEEP when the round ends. Delete public guest assets/copied binaries
then; preserve evidence and shared provider profiles. No CI or publication runs.
