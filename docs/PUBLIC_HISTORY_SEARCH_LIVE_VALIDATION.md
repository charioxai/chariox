# MP-08 / MP-10 / MP-11 — A09 real local drills (rounds 4–5)

MP-08 / MP-10 / MP-11: the requested local real Codex/TUI recall, privacy and
search-quality cells PASS. Broader hosted, worker, managed Path-1 and
browser/display acceptance is not established. No mock is counted as acceptance.

## MP-08 / MP-10 / MP-11: round 5 real RED → GREEN

Base: `1be2719e8` (kernel SHA-256 `4a3794897feaa4aaf2a024a43099bc3c3453a079b25bba22083806d2bf3bd994`,
Rust tree identical). Candidate: native kernel built from clean
`8e3164f0b3ec185662b989faaea334172c19dd37`, SHA-256
`b77b16fde8e200c02f5f0391e2a49a4a6480a189aaef5e90428fff8f6fa8f477`. The TUI
(SHA-256 `8511305e…dabf`) is unchanged. Official Codex used the product-linked
login; credentials were never read. Fresh kernel and TUI per scenario.

| Real user flow | Base | Candidate |
| --- | --- | --- |
| Agent stores a Vault value by hidden input, no Room Environment, then answers again | RED: every later answer rendered only the withheld marker (154 withheld events); peer recall empty. | PASS: answer visible in the TUI, 0 withheld events; peer `history.search`/`history.read` return the complete answer. |
| Peer searches ordinary words from a streamed review | RED: `stale cursor`, `cursor revision`, `stale cursor error` found no answer. | PASS: the 195-delta review is one 1,061-character public message; each query found it exactly once; read and turn return it complete. `search pagination` found no answer because the actual answer lacks `search`. |
| Official filesystem MCP registered as `files` reads a TEST file | RED: output persisted in 3 public rows; search returned 6 tool hits. | PASS: 0 public rows, 0 hits; both call references and a guessed reference return the same unavailable error. |
| Vault creation while a history read snapshot is open | RED: tool reported a stale-cursor error after the deletion committed. | PASS: tool reports `stored`; WAL truncation is deferred with a warning; matching public rows 2 → 0, 15 unrelated rows remain. |

Source checks at `8e3164f0b`: public history 35, Room protection 47 and protocol
226 tests PASS. Evidence: `round5`, `round5-red*`, `round5-wal-red` and
`round5-green-*` under the evidence root below.

## MP-08 / MP-10 / MP-11: round 4 exact source and clients

Assigned base: `59f9b312d8d30ff43378d12bbdd77f6a0477033e` (PR #912).
The real baseline kernel was built from
`4fbadc8624b8a7bddb3e8f2396e1833bd2fdff73`; every runtime dependency tree
listed in `round4/base-source-identities.json` equals the assigned base.
Its SHA-256 is
`69eb2460697dd6a19eb05b73b380984b49e64f6a1fe1568682efa7ea2a6834c0`.
The checkout inspected by the baseline provider was a later candidate; that does
not change the recorded baseline runtime identity.

The actual candidate native kernel was built and copied within one held compile
slot from `82f61518cf5c0c3dfa98c74c765fe52a3df938c2`, SHA-256
`4a3794897feaa4aaf2a024a43099bc3c3453a079b25bba22083806d2bf3bd994`.
It loaded the stock external browser controller from
`d9e0eeb08b52679281e3288023560aa141acbc0d`; only the controller resource
module and its test changed after that native build. The Rust and CLI trees are
unchanged. The actual compiled TUI SHA-256 is
`8511305e4a202ffe8b0158eee0cfd65b1efadf47f99b535c71c69905e989dabf`.
Final documentation commits do not relabel these binaries as newly built.
Local daemon 453 / relay 73 and all serialized shapes remain unchanged.

Both runs linked the existing authorized Codex login through normal
`/provider accounts link` and refresh flows, then ran the real official provider.
Credential files were never read, printed or copied. No 401/revocation occurred.
Actual peer agents were `baserecall/agent-2` and `recallpeer/agent-2`.
Tool results were captured through expanded real TUI tool output, alongside
terminal screenshots and console captures; internal fixtures are supplementary.

## MP-08 / MP-10 / MP-11: round 4 real RED → GREEN

| Real user flow | Baseline result | Candidate result |
| --- | --- | --- |
| Peer recalls a useful repository review | RED: `history.read` returned one streamed fragment rather than the answer. | PASS: the peer read the complete 954-character masked review assembled from 166 sanitized deltas. |
| Peer selects the preceding logical turn after a newer long review | RED: 411 newer output events; `turns_back:1` selected the latest review prompt. | PASS: 476 newer output events; offset and canonical reference selected the preceding prompt. Bounded `limit:1` canonical and native aliases returned the same complete answer. |
| Official private MCP call | The already-published private-title fix excluded both call records. | PASS: official filesystem MCP `read_text_file` executed against lane scratch. A query for its output-only TEST witness returned 0 hits; both call references and a guessed reference returned the identical unavailable error. |
| Store an already-public TEST word as an encrypted Vault value | RED: normal successful hidden-input creation left 8 public source matches and the earlier reference available. | PASS: normal successful creation reduced 15 earlier public matches to 0; original-value FTS count 0. Peer search returned 0 hits, and the earlier answer/prompt references became indistinguishable from a guessed reference. |

Round 4 searched individual fragments and assembled them on read; round 5
indexes the whole message instead (above).

The private package was the real official
`@modelcontextprotocol/server-filesystem@2026.8.31`, installed in lane-owned
scratch, registered with `/mcp install am9_vault_fs`, and granted to the actual
agent. Its complete normalized private call records were absent from public
source and detail. The drill does not claim that arbitrary later public echoes of
an unregistered private tool's output are classified as secret.

The TEST credential `am9_round4_test` was created through the product's encrypted
Vault flow with hidden passphrase/value input, browser use, and Room metadata.
Its value was never printed; retained evidence masks earlier public occurrences
of that TEST word. Registration now holds the existing public-history projection
mutex through successful storage and protection, invalidates earlier sanitized
rows/FTS, and registers Vault provenance before releasing the fence. Protected
Rooms retain the existing stronger withholding of provider streams. The actual
post-creation search's user-visible query was itself redacted.

## MP-08 / MP-10 / MP-11: normal Room Environment

Normal `/room start` and `/room status` ran in the same disposable home kernel.
The real kernel and stock Node controller shared a lane-owned OS PID/mount
namespace with real Chromium 147 on the same machine and filesystem. Codex used
its normal product-linked profile. No health response or process identity was
spoofed; this is an ordinary home-kernel leg, not a managed worker/slice drill.

Actual Room status: browser controller and browser **ready**, Wikipedia Main Page
loaded, input available, 1280×800 CSS viewport at scale 1. Lifecycle remained
`starting`; desktop/streamer and viewer were unavailable because no headed slice
was bound. This establishes the Room context used above, not full display,
public-site coverage, DPR 2, performance or stability acceptance.

The first real Chromium inventory attempt failed because Chromium rewrites
`/proc/PID/cmdline` into a joined process title. The shared resource observer now
parses flag clauses as data, preserving profile spaces and excluding renderer
processes. Real stock product argument order then observed one browser/profile
and passed health. Separate flag syntax and literal shell characters have
regressions; no shell evaluation is involved. Ambiguous arbitrary process titles
are not claimed to be supported.

## MP-08 / MP-10 / MP-11: supplementary checks and evidence

| Check | Result |
| --- | --- |
| Regression-only source `e680eb15e3a59c2b041e2b970f34e0536a233b32` | 24 PASS / 2 FAIL, exit 101: fragmented recall and logical-turn selection; production identical to assigned base. |
| Exact kernel source `82f61518c` | 29 public-history / 183 protocol checks PASS; native kernel build PASS. |
| Joined Chromium inventory fail-first | 2 PASS / 2 FAIL before correction; 4 PASS after. |
| Candidate controller/resource suites `d9e0eeb08` | 17 PASS. |
| Focused Vault lifecycle suite |45 PASS at exact source `d9e0eeb08`; exit 0, no tracked survivors. |

The initial regression overlay failed to compile due ambiguous string inference;
it is not the behavioral RED. Intermediate Vault-test setup incorrectly requested
ProcessMemory, then a split-secret assertion expected readable output despite the
existing stronger withholding policy. Both test mistakes were corrected without
weakening that policy; logs retain those failures.

Evidence root: `/root/.codex/evidence/browser-resume-20260930/am9/`.
`round4-red` and `round4-green` contain actual TUI step transcripts, expanded
provider tool JSON, masked terminal screenshots and console captures. `round4`
contains source/binary identities, source-test logs, count-only index diagnostics,
real peer/Vault/private-MCP verification, Room status, resource and cleanup
receipts. `final-audit.json` records the final documentation head separately.
Earlier unmasked TEST-word screenshots/ANSI were removed; replacements and the
redaction inventory are recorded. No Vault/provider payload was read for the
count-only diagnostics. All generated state remains outside source.

## MP-08 / MP-10 / MP-11: remaining acceptance scope

Round 3's local room-scoping and 13-page concurrent increasing-append pagination
PASS remain historical results with their original identities. Rounds 4–5 do not
relabel them as candidate runs. Delayed leased lower-sequence append, hosted wss,
worker/slice and managed Path-1, Claude/OpenCode, other clients, real public
site/service matrix, DPR 1/2, shaped uplink/RTT, latency/fps and multi-hour stability
remain unestablished. The coordinator must provide the integrated approved
stack, account/client/machine resources and authorize hosted execution; this lane
was expressly forbidden to contact or deploy those services.

MP-11 current semantic review is required for history admission/observation
protection and the browser resource inventory anchor. Non-security source is not
an exact-blob review blocker under the narrowed rule. No entire MP item is closed
by these local results. No push, GitHub CI, deployment or publication was run.
