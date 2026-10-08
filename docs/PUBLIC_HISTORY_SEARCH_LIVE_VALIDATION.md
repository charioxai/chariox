# MP-08 / MP-10 / MP-11 — A09 real live drills (rounds 4–7)

MP-08 / MP-10 / MP-11: the round 8 review removes the stale #917 merge
`4be64ca26` and its `0bbacdd0fa` ancestor from #912. History search does not
depend on that enrollment snapshot. Claude setup-token enrollment and alias
validation belong to the separate current #917 (`81d7ecf13`), integrated for
those drills. The round 7 receipts below retain their original source identities
and do not establish acceptance of the clean review head. The hosted stability
cell is **NOT-RUN (#927 on main)** for this round; its earlier RED is retained.

MP-08 / MP-10 / MP-11: round 7 real OpenCode, Codex, Claude `-p` and Claude
headless recall cells PASS. Hosted web at DPR 1/2 and remote TUI recall also
PASS over shaped WSS. Hosted multi-hour stability is **RED**; the leased-worker
hosted cell fails before spawn at metadata discovery. Overall A09 acceptance
remains on hold. Earlier rounds below retain their original source identities.

## MP-08 / MP-10 / MP-11: round 7 source and checks

Assigned base: `1e7468fd44686f4fe5de71c95897dc684cffed2c`.
Corrected kernel and relay were built from
`86d5cec19689d5adbf0f5b8eeb3168b679a16fb3`; documentation/runtime results do
not relabel those artifacts as later heads. `2c197bea1` has the identical Git
tree. Daemon 453 and relay 73 wire shapes are unchanged. The existing #917
prerequisite was merged to use product-supported Claude setup-token enrollment.
No shared provider login was changed, logged out, revoked or read directly.

Source checks PASS: public history 43, Room protection 47, operational history 20,
OpenCode 111, account credentials 7, context resolver 1 and Claude setup-token 13
(242 total). `cargo fmt --check`, clippy and exact native kernel/relay builds
exit 0. Cargo ran through coordinator slots with disk-backed TMPDIR; compiler
JSON verified critical crate manifests belonged to this checkout and were freshly
compiled. A separate foreign-cache compile failure is not a behavioral RED.

The real TUI client uses the frozen Solid 1.9.11 and OpenTUI 0.1.87 patch. A
scratch install with two Solid runtimes produced an invalid startup result;
a speculative CLI workaround was reverted. The corrected client has no net
source change. `round7/unified-client-provenance.json` distinguishes the earlier
unpatched client from the final patched build. The final remote TUI and local
leased-provider repetitions used the patched client.

## MP-08 / MP-10 / MP-11: round 7 real cells

| Real user flow | Result and limit |
| --- | --- |
| OpenCode peer recalls a repository review with search/read/turn | Base RED: official OpenCode 1.18.23 hangs in its Linux native watcher; four server routes time out. Same base/account with only the official watcher flag completes in 20.085 s. Production fix, without parent flag injection, PASS 16.265 s prompt-to-final-output (32.173 s complete spawn/recall phase). |
| Claude `-p` root review and peer search/read/turn | PASS with the fleet account enrolled through the product setup-token flow, encrypted disposable Vault and hidden passphrase. Search snippets are bounded; read/turn contain the complete review. |
| Claude headless root review and peer recall | PASS, including the real workspace-trust RuntimeInteraction answered through the TUI. No approval bypass. |
| Codex Vault registration while a SQLite read snapshot is held | PASS: hidden disposable passphrase/value; matching public rows2→0, no TEST value remains in public source; later answers remain visible and peer recall reads them. |
| Real local OpenCode home run and Codex leased worker share raw `provider-run-1` | Base RED: actual Codex worker tool records are stamped OpenCode through the colliding raw counter. Corrected home stamps Codex; genuine native Git commands are searchable and peer read returns the six-sentence OpenCode review with `truncated:false`. Local relay/provider evidence supplements the hosted placement requirement. |
| Hosted web, real app entry, DPR 1 and DPR 2 | PASS fresh uniquely marked Codex recalls: 10.212 s / 11.301 s. Exact kernel selection is ONLINE with fresh heartbeat, selected through the waiting room. |
| Hosted remote TUI, final frozen client dependencies | PASS fresh Codex peer search/read: 14.642 s, actual TUI exit 0. Earlier unpatched repetition 19.165 s is separately recorded. |
| Hosted two-hour browser stability | RED after about 14 min: two control sockets close after the last good 13.01 min sample; no new shaper failure. Do not count as a multi-hour pass. |
| Hosted leased-worker spawn | RED before execution: `decode relay metadata response: relay metadata response does not match request`. No worker-run acceptance established. |

The hosted cells use own `pr912-am9`, Cloud
`22411adb14c514882bb6412835fa3575de335316`, exact corrected OSS and its relay,
trusted HTTPS/WSS, real Chromium and real provider accounts. The opaque shaper
configured 8 Mbit/s / 30 ms each way, measured 7.77 Mbit/s calibration and real web
relay RTT at least 66 ms. Remote TUI echo calibration RTT 61–63 ms is reported
separately from the web's authenticated relay round-trip samples. No TLS payload
or credentials were inspected. An initial navigation EPIPE is retained in the
network evidence; soak failure counters were never reset to hide a closure.

## MP-08 / MP-10 / MP-11: round 7 security and acceptance limits

The reviewer P1 is reproduced through the actual admitted remote projection and
fan-out with colliding home OpenCode / worker Codex run IDs. Before the fix, the
worker's normalized private MCP record enters public history (test exit101).
After the fix, its whole record/reference is unavailable and the genuine worker
native command remains searchable. Production resolves provider family from the
current admitted lease projection with exact session/agent/run binding, never
from a colliding local counter. Index 4 fences/removes prior public projections
without importing raw history. This actual-path regression is supplementary to
real provider drills, not a claim of every live private-MCP placement passing.

The own hosted relay log contains six token-expiry closure events; its JSON
logger omits timestamps, so those events alone do not prove temporal correlation
with the soak. Source inspection confirms that relay sockets close at token
expiry and this Cloud baseline refreshes bootstrap before opening a new socket.
The soak remains RED. The coordinator needs to supply compatible admission
renewal integration (and allocate protocol changes if needed) and resolve the
hosted metadata-discovery baseline. No expiry extension or admission bypass was
used. Optional `Notes` / `KernelBrowser` requests from this Cloud are unsupported
by the old 453 kernel and retained in diagnostics.

Evidence: `round7/`, `round7-unified-green-opencode/`,
`round7-ready-green-claude/{claude-p,claude-headless}/`,
`round7-final-green-codex/`, `round7-hosted-final-codex/`,
`round7-lease-green-home/` and `round7-lease-base-home/` under the evidence root.
They contain exact build identities, commands/exit codes, per-step terminal and
browser screenshots, masked UI captures, timings, resource samples and cleanup
receipts. No whole MP item closes. Real public-site display coverage, click/fps
thresholds, managed/slice placements and remaining A09 cells are not established
by these history recall results. MP-11 changed security anchors are listed in
the external lane handoff for the independent reviewer; non-security source is
not an exact-blob review blocker. No CI or publication was run.

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

## MP-08 / MP-10 / MP-11: historical round 4–5 acceptance scope

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
