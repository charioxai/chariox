# MP-08 / MP-10 / MP-11 — A09 round 3 real local Codex drills

MP-08 / MP-10 / MP-11: this round is **not accepted**. The real built TUI,
native kernel and official Codex completed useful repository reviews through
normal product prompt and MCP paths. Local scope and bounded pagination passed;
useful answer recall and previous-turn selection failed. Protected-input and
private-service acceptance require resources listed below. No synthetic test is
counted as live acceptance.

## MP-08 / MP-10 / MP-11: identities and readiness

The initial live candidate was `c639141184617c1380a47dde4a8502f3c0c8cc69`
(local daemon453 / relay73). Its retained native kernel SHA-256 was
`3ff3bd617a6e7588ef2d9d2d63ac1594f5ed537c2b216af3b777bee271870dc9`.
The actual compiled TUI SHA-256 was
`8511305e4a202ffe8b0158eee0cfd65b1efadf47f99b535c71c69905e989dabf`;
its CLI tree was unchanged from the previously recorded protocol453 build.

Owner authorization was exercised through the real TUI:
`/provider accounts link codex am9-b2 /root/.codex-agents`, then product refresh,
configured/authenticated status and actual Codex execution. Credential files were
never read, printed or copied. No revocation/401 was observed. This readiness
supersedes round2's dedicated-profile blocker; it supplies no protected-service
credential or hosted/worker account.

For the pre-feature catalog RED, the native kernel was checksum-proven source
`52a18a46eebad297caad5ab7a6fa662fc75ad868`, SHA-256
`4ee3afef157e4a9d20c05185e1f378bf5231d889df22bafc8f483ed438bfb998`.
Its real TUI was separately built from `fa2a5bcad796923f180fc80225b549bf182e9eda`.
These distinct identities are recorded rather than attributed to the candidate.

## MP-08 / MP-10 / MP-11: actual results

| A09 scenario | Result and evidence |
| --- | --- |
| Room-authorized history | Local PASS: actual same-room search/read; explicit existing foreign-room search denied without live sudo. A valid foreign reference and a guessed reference both returned the same unavailable error. |
| Pagination during concurrent increasing appends | Local PASS: two real Codex reviews; all13 frozen hits returned once in13 one-hit pages, unchanged cursors, sequence ceiling1397, then null. The other review appended678 output records through sequence2408. A scoped reference/sequence-only diagnostic independently matched the exact frozen result set. |
| Useful earlier peer answer | RED: returned answer reference read only `_cursor`, while the real TUI displayed the useful review. The public projection indexes streamed provider output as separate token fragments. The captured actual MCP read, not just the provider's interpretation, establishes this seam. |
| Previous retained turn after more than200 events | RED: `turns_back:1` returned the latest review's prompt, not the preceding scope-check turn. Exact `turn_ref` selected the same wrong group. The user prompt uses a prompt ID as `turn_id`;678 output records use the provider-native turn ID for that same prompt. One logical turn is split into two groups. Missing turns returned0 events. |
| Secrets never indexed/returned | BLOCKED: product `/credential list` reports0 credentials; `/room status` reports no Room Environment. No owner-approved real private MCP service/account is supplied. |
| Invalidation after protected redaction | BLOCKED at that same real protected-service/input prerequisite. |
| Pre-feature provider catalog | RED: actual official Codex reports that history.search is absent; only read/turn are available. |

Evidence root is `/root/.codex/evidence/browser-resume-20260930/am9/`.
`round3-green`, `round3-home` and `round3-base` hold step screenshots,
ANSI/console captures and actual TUI automation transcripts. `round3` holds
identity receipts, extracted actual MCP records, pagination verification,
canonical prompt/native-turn binding counts, resource samples and cleanup.
Provider tool results were captured from user-visible TUI output; direct IPC and
synthetic providers were not substituted for these live steps.

An early multi-room catch-up attachment failed; the stable second real TUI
completed the scoped and pagination drills. Root cause for that catch-up failure
is not established. One restart changed the kernel listener port, changing its
normal bind-scoped product identity. That harness mistake establishes neither a
restart-persistence failure nor a persistence pass; replay preserves the port.

## MP-08 / MP-11: private MCP review correction

Review inbox05:05 identified a P1: Codex normalizes a generic MCP `read` with
server identity in `title`, but exclusion examined only tool names. The added
regression uses that actual normalized shape and an unregistered synthetic
canary, requiring no public search/detail record while preserving harmless
public document reads. It is a source regression, not a live private-service run.

The correction classifies `title`, `server` and `server_name` alongside tool
identity. Public projection version2 fences/removes predecessor version1 rows,
so an upgrade cannot retain already admitted private output. Raw records are
never backfilled. Wire shapes and daemon453/relay73 remain unchanged.

Fail-first and focused GREEN check receipts are recorded with exact source and
binary identities in the external round3 evidence and lane status. Those checks
do not close the live private-service blocker.

## MP-08 / MP-10 / MP-11: remaining acceptance and cleanup

Owner action: supply an approved real private MCP service/capability and account
through normal product context, a real-service credential through Chariox Vault,
and a reviewed Room Browser/Computer Environment. Repeat private-record exclusion
and protected-input/redaction invalidation through the actual TUI/provider.
No credential should be copied into a prompt or evidence.

Coordinator action: correct answer coalescing and logical prompt/native-turn
grouping, then repeat the two actual RED scenarios. Approved hosted wss, worker,
managed Path-1, other provider/client axes, real public sites/DPR1/2, shaped
network timings and multi-hour stability remain unestablished. Increasing local
appends do not establish delayed lower-sequence leased-append acceptance.

Owned real TUI/kernel/provider descendants exited; exact disposable runtime
state and its generated identities were inventoried and removed without following
links to the shared login. No Docker resources were created. Shared reviewer,
other lanes, provider login and shared build cache were preserved. Resource and
final cleanup receipts remain outside the repository.
